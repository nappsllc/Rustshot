//! Minimal hand-rolled Objective-C support for the macOS recorder
//! (ScreenCaptureKit, AVFoundation), without crates:
//!
//! - [`Block`]: a block literal in the clang block ABI, for completion
//!   handlers. It is laid out like a *stack* block (`_NSConcreteStackBlock`
//!   with copy/dispose helpers and a type signature): a callee that keeps it calls `Block_copy`,
//!   which moves a copy to the heap and runs our copy helper (one more
//!   reference to the shared closure); `Block_release` of that copy runs
//!   the dispose helper. Our own literal lives until [`Block`] drops, which
//!   must not be before the message that takes it returns. Arguments reach
//!   the closure as machine words (`usize`): every argument used here is a
//!   pointer, an integer or a `BOOL` (read its low byte).
//! - Delegate classes made at run time ([`make_class`]) with one pointer
//!   ivar for the Rust context ([`set_ctx`] / [`ctx`]).
//! - Dispatch queues, autorelease pools, `dlopen`, Foundation values.
//!
//! The block layout and the helpers' reference counting are tested on
//! every platform; the rest is macOS only.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use core::ffi::c_void;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------

/// `BLOCK_HAS_COPY_DISPOSE`: the descriptor has copy and dispose helpers.
pub const BLOCK_HAS_COPY_DISPOSE: i32 = 1 << 25;
/// `BLOCK_HAS_SIGNATURE`: the descriptor has the block's type signature
/// (after the helpers), for callees that inspect it (NSInvocation,
/// `-[NSBlockOperation ...]`, some framework argument checks).
pub const BLOCK_HAS_SIGNATURE: i32 = 1 << 30;

/// `Block_descriptor_1` + `Block_descriptor_2` + `Block_descriptor_3`.
#[repr(C)]
pub struct BlockDesc {
    reserved: usize,
    size: usize,
    copy: unsafe extern "C" fn(dst: *mut BlockLit, src: *const BlockLit),
    dispose: unsafe extern "C" fn(b: *const BlockLit),
    /// ObjC type encoding of the invoke function (`v<size>@?0...`).
    signature: *const core::ffi::c_char,
    /// Extended layout (unused: no `BLOCK_HAS_EXTENDED_LAYOUT`).
    layout: *const core::ffi::c_char,
}

// Immutable statics: function pointers and C string literals.
unsafe impl Sync for BlockDesc {}

/// `Block_layout` with our one captured variable: the shared closure.
#[repr(C)]
pub struct BlockLit {
    isa: *const c_void,
    flags: i32,
    reserved: i32,
    invoke: *const c_void,
    desc: *const BlockDesc,
    ctx: *const Ctx,
}

type Body = dyn Fn(&[usize]) + Send + Sync;

/// What every copy of a block shares (one `Arc` reference per copy).
struct Ctx(Box<Body>);

const fn desc(signature: &'static core::ffi::CStr) -> BlockDesc {
    BlockDesc {
        reserved: 0,
        size: std::mem::size_of::<BlockLit>(),
        copy: copy_helper,
        dispose: dispose_helper,
        signature: signature.as_ptr(),
        layout: core::ptr::null(),
    }
}

/// One descriptor per arity: `void (^)(void)`, `void (^)(word)`,
/// `void (^)(word, word)`; the words are typed `^v` (any pointer-sized
/// argument: objects, NSInteger; a BOOL is widened by the caller).
static DESCS: [BlockDesc; 3] = [desc(c"v8@?0"), desc(c"v16@?0^v8"), desc(c"v24@?0^v8^v16")];

/// `Block_copy` moved `src` to `dst` (bytes copied): `dst` holds a reference too.
unsafe extern "C" fn copy_helper(dst: *mut BlockLit, src: *const BlockLit) {
    unsafe {
        let ctx = (*src).ctx;
        Arc::increment_strong_count(ctx);
        (*dst).ctx = ctx;
    }
}

/// A heap copy is released.
unsafe extern "C" fn dispose_helper(b: *const BlockLit) {
    unsafe { Arc::decrement_strong_count((*b).ctx) }
}

unsafe fn call(b: *const BlockLit, args: &[usize]) {
    unsafe {
        let ctx = &*(*b).ctx;
        (ctx.0)(args)
    }
}

unsafe extern "C" fn invoke0(b: *const BlockLit) {
    unsafe { call(b, &[]) }
}

unsafe extern "C" fn invoke1(b: *const BlockLit, a: usize) {
    unsafe { call(b, &[a]) }
}

unsafe extern "C" fn invoke2(b: *const BlockLit, a: usize, c: usize) {
    unsafe { call(b, &[a, c]) }
}

#[cfg(target_os = "macos")]
unsafe extern "C" {
    #[allow(non_upper_case_globals)]
    static _NSConcreteStackBlock: [*const c_void; 32];
}

fn stack_isa() -> *const c_void {
    #[cfg(target_os = "macos")]
    {
        (&raw const _NSConcreteStackBlock).cast()
    }
    #[cfg(not(target_os = "macos"))]
    {
        core::ptr::null()
    }
}

/// A block taking `arity` (0..=2) word-sized arguments that runs `f`
/// with them. See the module docs for its lifetime.
pub struct Block {
    lit: Box<BlockLit>,
}

// The closure is Send + Sync; the literal is only read.
unsafe impl Send for Block {}

impl Block {
    pub fn new(arity: usize, f: impl Fn(&[usize]) + Send + Sync + 'static) -> Block {
        let arity = arity.min(2);
        let invoke = match arity {
            0 => invoke0 as *const c_void,
            1 => invoke1 as *const c_void,
            _ => invoke2 as *const c_void,
        };
        let ctx = Arc::into_raw(Arc::new(Ctx(Box::new(f))));
        Block {
            lit: Box::new(BlockLit {
                isa: stack_isa(),
                flags: BLOCK_HAS_COPY_DISPOSE | BLOCK_HAS_SIGNATURE,
                reserved: 0,
                invoke,
                desc: &DESCS[arity],
                ctx,
            }),
        }
    }

    /// The block, to pass as an argument.
    pub fn as_ptr(&self) -> *mut c_void {
        (&*self.lit as *const BlockLit).cast_mut().cast()
    }
}

impl Drop for Block {
    fn drop(&mut self) {
        unsafe { Arc::decrement_strong_count(self.lit.ctx) }
    }
}

/// A one-shot completion handler: the block, and the receiver of what its
/// closure sends (`f` maps the block's arguments to the value).
pub fn completion<T: Send + 'static>(
    arity: usize,
    f: impl Fn(&[usize]) -> T + Send + Sync + 'static,
) -> (Block, crate::chan::Receiver<T>) {
    let (tx, rx) = crate::chan::channel();
    let tx = std::sync::Mutex::new(Some(tx));
    let b = Block::new(arity, move |a| {
        if let Some(tx) = tx.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(f(a));
        }
    });
    (b, rx)
}

// ---------------------------------------------------------------------------
// macOS: runtime, Foundation, dispatch
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
pub use mac::*;

#[cfg(target_os = "macos")]
mod mac {
    use crate::wind::{msg0, msg1, msg2, msg3, objc_cls, objc_sel};
    use core::ffi::{c_char, c_int, c_void, CStr};

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_allocateClassPair(superclass: *mut c_void, name: *const c_char, extra: usize) -> *mut c_void;
        fn objc_registerClassPair(class: *mut c_void);
        fn class_addMethod(class: *mut c_void, sel: usize, imp: *const c_void, types: *const c_char) -> i8;
        fn class_addIvar(class: *mut c_void, name: *const c_char, size: usize, align: u8, types: *const c_char) -> i8;
        fn class_addProtocol(class: *mut c_void, proto: *mut c_void) -> i8;
        fn objc_getProtocol(name: *const c_char) -> *mut c_void;
        fn class_getInstanceVariable(class: *mut c_void, name: *const c_char) -> *mut c_void;
        fn ivar_getOffset(ivar: *mut c_void) -> isize;
        fn object_getClass(obj: *mut c_void) -> *mut c_void;
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    unsafe extern "C" {
        fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> *mut c_void;
        fn dispatch_sync_f(q: *mut c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
        fn dispatch_async_f(q: *mut c_void, ctx: *mut c_void, work: extern "C" fn(*mut c_void));
        fn dispatch_release(obj: *mut c_void);
        static _dispatch_main_q: u8;
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn pthread_main_np() -> c_int;
    }

    #[cfg(target_arch = "x86_64")]
    #[link(name = "objc")]
    #[cfg_attr(not(test), allow(dead_code))]
    unsafe extern "C" {
        fn objc_msgSend_stret();
    }

    /// A message without arguments returning a struct of more than 16
    /// bytes (a `CMTime`): x86-64 returns it through a hidden pointer,
    /// which needs `objc_msgSend_stret`; arm64 uses plain `objc_msgSend`.
    #[cfg_attr(not(test), allow(dead_code))] // the live test
    pub unsafe fn msg_big0<R: Copy>(recv: *mut c_void, sel: usize) -> R {
        unsafe {
            #[cfg(target_arch = "x86_64")]
            {
                let f: unsafe extern "C" fn(*mut c_void, usize) -> R =
                    core::mem::transmute(objc_msgSend_stret as unsafe extern "C" fn());
                f(recv, sel)
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                msg0(recv, sel)
            }
        }
    }

    const IVAR: &CStr = c"rsCtx";

    /// An NSObject subclass `name` with one pointer ivar for the Rust
    /// context, conforming to `protocols` (those the runtime knows) and
    /// implementing `methods` (selector, function, type encoding). Made
    /// once; later calls return the registered class.
    ///
    /// Each name is made under its own `OnceLock` (a registry keyed by the
    /// name): a second thread asking for the same class waits for the
    /// first to register it, never sees it half-built.
    pub fn make_class(name: &CStr, protocols: &[&CStr], methods: &[(&CStr, *const c_void, &CStr)]) -> *mut c_void {
        use std::collections::HashMap;
        use std::sync::{Mutex, OnceLock};
        static CLASSES: Mutex<Option<HashMap<std::ffi::CString, &'static OnceLock<usize>>>> = Mutex::new(None);
        let once: &'static OnceLock<usize> = {
            let mut m = CLASSES.lock().unwrap_or_else(|e| e.into_inner());
            m.get_or_insert_with(HashMap::new)
                .entry(name.to_owned())
                .or_insert_with(|| Box::leak(Box::new(OnceLock::new())))
        };
        *once.get_or_init(|| build_class(name, protocols, methods) as usize) as *mut c_void
    }

    fn build_class(name: &CStr, protocols: &[&CStr], methods: &[(&CStr, *const c_void, &CStr)]) -> *mut c_void {
        unsafe {
            let have = objc_cls(name);
            if !have.is_null() {
                return have;
            }
            let cls = objc_allocateClassPair(objc_cls(c"NSObject"), name.as_ptr(), 0);
            if cls.is_null() {
                return objc_cls(name);
            }
            class_addIvar(cls, IVAR.as_ptr(), 8, 3, c"^v".as_ptr());
            for p in protocols {
                let proto = objc_getProtocol(p.as_ptr());
                if !proto.is_null() {
                    class_addProtocol(cls, proto);
                }
            }
            for (sel, imp, types) in methods {
                class_addMethod(cls, objc_sel(sel), *imp, types.as_ptr());
            }
            objc_registerClassPair(cls);
            cls
        }
    }

    unsafe fn ivar_slot(obj: *mut c_void) -> Option<*mut *const c_void> {
        unsafe {
            if obj.is_null() {
                return None;
            }
            let iv = class_getInstanceVariable(object_getClass(obj), IVAR.as_ptr());
            if iv.is_null() {
                return None;
            }
            Some(obj.cast::<u8>().offset(ivar_getOffset(iv)).cast())
        }
    }

    /// Store the context pointer of an object of a [`make_class`] class.
    pub unsafe fn set_ctx(obj: *mut c_void, p: *const c_void) {
        unsafe {
            if let Some(s) = ivar_slot(obj) {
                *s = p;
            }
        }
    }

    /// The context pointer stored with [`set_ctx`] (null if none).
    pub unsafe fn ctx(obj: *mut c_void) -> *const c_void {
        unsafe { ivar_slot(obj).map_or(core::ptr::null(), |s| *s) }
    }

    /// An autorelease pool for the current scope (threads we start have
    /// none; objects autoreleased there would pile up until they exit).
    pub struct Pool(*mut c_void);

    impl Pool {
        pub fn new() -> Pool {
            Pool(unsafe { objc_autoreleasePoolPush() })
        }
    }

    impl Drop for Pool {
        fn drop(&mut self) {
            unsafe { objc_autoreleasePoolPop(self.0) }
        }
    }

    /// An owned (+1) Objective-C object, released on drop.
    pub struct Id(pub *mut c_void);

    // Objective-C reference counting is thread-safe; what is called on
    // the object from which thread is the owner's business.
    unsafe impl Send for Id {}
    unsafe impl Sync for Id {}

    impl Id {
        /// Take an object we own (alloc/init, new, copy, Create).
        pub fn owned(p: *mut c_void) -> Option<Id> {
            (!p.is_null()).then(|| Id(p))
        }

        /// Retain an object we do not own (an autoreleased result).
        pub fn retain(p: *mut c_void) -> Option<Id> {
            if p.is_null() {
                return None;
            }
            unsafe {
                let _: *mut c_void = msg0(p, objc_sel(c"retain"));
            }
            Some(Id(p))
        }

        pub fn ptr(&self) -> *mut c_void {
            self.0
        }
    }

    impl Drop for Id {
        fn drop(&mut self) {
            unsafe {
                let _: () = msg0(self.0, objc_sel(c"release"));
            }
        }
    }

    /// `[[cls alloc] init]`.
    pub unsafe fn new_obj(cls: *mut c_void) -> Option<Id> {
        unsafe {
            if cls.is_null() {
                return None;
            }
            let o: *mut c_void = msg0(cls, objc_sel(c"alloc"));
            if o.is_null() {
                return None;
            }
            Id::owned(msg0(o, objc_sel(c"init")))
        }
    }

    /// A serial dispatch queue, released on drop.
    pub struct Queue(*mut c_void);

    unsafe impl Send for Queue {}
    unsafe impl Sync for Queue {}

    extern "C" fn nothing(_: *mut c_void) {}

    impl Queue {
        pub fn serial(label: &CStr) -> Queue {
            Queue(unsafe { dispatch_queue_create(label.as_ptr(), core::ptr::null()) })
        }

        pub fn ptr(&self) -> *mut c_void {
            self.0
        }

        /// Wait until what was queued before has run (a callback running on
        /// it now is over when this returns).
        pub fn drain(&self) {
            unsafe { dispatch_sync_f(self.0, core::ptr::null_mut(), nothing) }
        }
    }

    impl Drop for Queue {
        fn drop(&mut self) {
            unsafe { dispatch_release(self.0) }
        }
    }

    /// Run `work(ctx)` on the main thread, from its run loop (the overlay
    /// pump waits in `nextEventMatchingMask`, which serves the main queue).
    pub fn on_main(ctx: *mut c_void, work: extern "C" fn(*mut c_void)) {
        unsafe { dispatch_async_f((&raw const _dispatch_main_q).cast_mut().cast(), ctx, work) }
    }

    /// Whether this is the main thread (AppKit windows live there).
    pub fn is_main_thread() -> bool {
        unsafe { pthread_main_np() != 0 }
    }

    /// `dlopen` (lazy, global); null when it cannot be loaded.
    pub fn load(path: &CStr) -> *mut c_void {
        unsafe { dlopen(path.as_ptr(), 0x1 | 0x8) }
    }

    /// `dlsym` (null when missing).
    pub fn sym(lib: *mut c_void, name: &CStr) -> *mut c_void {
        if lib.is_null() {
            return core::ptr::null_mut();
        }
        unsafe { dlsym(lib, name.as_ptr()) }
    }

    /// The value of an `NSString *const` exported by a loaded image (null
    /// when missing).
    pub unsafe fn const_obj(lib: *mut c_void, name: &CStr) -> *mut c_void {
        let p = sym(lib, name) as *const *mut c_void;
        if p.is_null() { core::ptr::null_mut() } else { unsafe { *p } }
    }

    /// The Rust string of an NSString ("" for nil).
    pub unsafe fn string_of(ns: *mut c_void) -> String {
        unsafe {
            if ns.is_null() {
                return String::new();
            }
            let p: *const c_char = msg0(ns, objc_sel(c"UTF8String"));
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        }
    }

    /// An NSError's localized description ("unknown error" for nil).
    pub unsafe fn error_text(err: *mut c_void) -> String {
        unsafe {
            if err.is_null() {
                return "unknown error".into();
            }
            let d: *mut c_void = msg0(err, objc_sel(c"localizedDescription"));
            let s = string_of(d);
            if s.is_empty() { "unknown error".into() } else { s }
        }
    }

    /// `[NSNumber numberWithLongLong:]` (autoreleased).
    pub unsafe fn num_i64(v: i64) -> *mut c_void {
        unsafe { msg1(objc_cls(c"NSNumber"), objc_sel(c"numberWithLongLong:"), v) }
    }

    /// `[NSNumber numberWithDouble:]` (autoreleased).
    pub unsafe fn num_f64(v: f64) -> *mut c_void {
        unsafe { msg1(objc_cls(c"NSNumber"), objc_sel(c"numberWithDouble:"), v) }
    }

    /// `[NSNumber numberWithBool:]` (autoreleased).
    pub unsafe fn num_bool(v: bool) -> *mut c_void {
        unsafe { msg1(objc_cls(c"NSNumber"), objc_sel(c"numberWithBool:"), i8::from(v)) }
    }

    /// An NSDictionary of `pairs` (key, value; autoreleased). Nil keys or
    /// values are left out.
    pub unsafe fn dict(pairs: &[(*mut c_void, *mut c_void)]) -> *mut c_void {
        let (keys, vals): (Vec<_>, Vec<_>) = pairs.iter().copied().filter(|(k, v)| !k.is_null() && !v.is_null()).unzip();
        unsafe {
            msg3(
                objc_cls(c"NSDictionary"),
                objc_sel(c"dictionaryWithObjects:forKeys:count:"),
                vals.as_ptr(),
                keys.as_ptr(),
                vals.len(),
            )
        }
    }

    /// An NSArray of `items` (autoreleased; nils left out).
    pub unsafe fn array(items: &[*mut c_void]) -> *mut c_void {
        let v: Vec<_> = items.iter().copied().filter(|p| !p.is_null()).collect();
        unsafe { msg2(objc_cls(c"NSArray"), objc_sel(c"arrayWithObjects:count:"), v.as_ptr(), v.len()) }
    }

    /// The objects of an NSArray.
    pub unsafe fn items(arr: *mut c_void) -> Vec<*mut c_void> {
        unsafe {
            if arr.is_null() {
                return Vec::new();
            }
            let n: usize = msg0(arr, objc_sel(c"count"));
            (0..n).map(|i| msg1(arr, objc_sel(c"objectAtIndex:"), i)).collect()
        }
    }

    /// Whether instances of class `cls` answer `sel` (an API of a newer
    /// macOS).
    pub unsafe fn has_method(cls: *mut c_void, sel: &CStr) -> bool {
        unsafe {
            if cls.is_null() {
                return false;
            }
            let r: i8 = msg1(cls, objc_sel(c"instancesRespondToSelector:"), objc_sel(sel));
            r != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[test]
    fn the_block_literal_has_the_clang_layout() {
        assert_eq!(size_of::<usize>(), 8, "64-bit targets only");
        assert_eq!(offset_of!(BlockLit, isa), 0);
        assert_eq!(offset_of!(BlockLit, flags), 8);
        assert_eq!(offset_of!(BlockLit, reserved), 12);
        assert_eq!(offset_of!(BlockLit, invoke), 16);
        assert_eq!(offset_of!(BlockLit, desc), 24);
        assert_eq!(offset_of!(BlockLit, ctx), 32);
        assert_eq!(size_of::<BlockLit>(), 40);
        // reserved, size, copy, dispose, signature, layout
        assert_eq!(offset_of!(BlockDesc, size), 8);
        assert_eq!(offset_of!(BlockDesc, copy), 16);
        assert_eq!(offset_of!(BlockDesc, dispose), 24);
        assert_eq!(offset_of!(BlockDesc, signature), 32);
        assert_eq!(offset_of!(BlockDesc, layout), 40);
        assert_eq!(size_of::<BlockDesc>(), 48);
        assert_eq!(BLOCK_HAS_COPY_DISPOSE, 0x0200_0000);
        assert_eq!(BLOCK_HAS_SIGNATURE, 0x4000_0000);
        let sig = |d: &BlockDesc| unsafe { core::ffi::CStr::from_ptr(d.signature) }.to_str().unwrap().to_owned();
        for (i, want) in ["v8@?0", "v16@?0^v8", "v24@?0^v8^v16"].iter().enumerate() {
            assert_eq!(DESCS[i].size, 40);
            assert_eq!(&sig(&DESCS[i]), want);
            assert!(DESCS[i].layout.is_null());
            let b = Block::new(i, |_| {});
            let l = unsafe { &*lit(&b) };
            assert_eq!(l.flags, BLOCK_HAS_COPY_DISPOSE | BLOCK_HAS_SIGNATURE);
            assert!(std::ptr::eq(l.desc, &DESCS[i]), "arity {i} gets its own signature");
        }
    }

    fn lit(b: &Block) -> *mut BlockLit {
        b.as_ptr().cast()
    }

    #[test]
    fn a_block_passes_its_arguments_to_the_closure() {
        let got = Arc::new(Mutex::new(Vec::new()));
        let g = got.clone();
        let b = Block::new(2, move |a| g.lock().unwrap().extend_from_slice(a));
        unsafe {
            let l = lit(&b);
            assert_eq!((*l).flags, BLOCK_HAS_COPY_DISPOSE | BLOCK_HAS_SIGNATURE);
            let f: unsafe extern "C" fn(*const BlockLit, usize, usize) = std::mem::transmute((*l).invoke);
            f(l, 7, 0x1_0000_0001); // a BOOL comes in the low byte
        }
        assert_eq!(*got.lock().unwrap(), vec![7, 0x1_0000_0001]);
        let b0 = Block::new(0, |a| assert!(a.is_empty()));
        unsafe {
            let l = lit(&b0);
            let f: unsafe extern "C" fn(*const BlockLit) = std::mem::transmute((*l).invoke);
            f(l);
        }
    }

    struct Counted(Arc<AtomicUsize>);

    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn a_heap_copy_keeps_the_closure_until_it_is_disposed() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let (c, k) = (Counted(dropped.clone()), calls.clone());
        let b = Block::new(1, move |a| {
            let _ = &c;
            k.fetch_add(a[0], Ordering::SeqCst);
        });
        // What Block_copy does: copy the bytes, then the copy helper.
        let heap: *mut BlockLit = Box::into_raw(Box::new(unsafe { std::ptr::read(lit(&b)) }));
        unsafe {
            let d = &*(*heap).desc;
            (d.copy)(heap, lit(&b));
        }
        drop(b); // our literal goes; the copy still works
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        unsafe {
            let f: unsafe extern "C" fn(*const BlockLit, usize) = std::mem::transmute((*heap).invoke);
            f(heap, 5);
            assert_eq!(calls.load(Ordering::SeqCst), 5);
            ((*(*heap).desc).dispose)(heap);
            drop(Box::from_raw(heap));
        }
        assert_eq!(dropped.load(Ordering::SeqCst), 1, "the last reference freed the closure");
    }

    #[test]
    fn a_completion_sends_once() {
        let (b, rx) = completion(1, |a| a[0] as u8 != 0);
        unsafe {
            let l = lit(&b);
            let f: unsafe extern "C" fn(*const BlockLit, usize) = std::mem::transmute((*l).invoke);
            f(l, 0xFF01);
            f(l, 0);
        }
        assert_eq!(rx.try_recv(), Ok(true));
        assert!(rx.try_recv().is_err(), "called twice, sent once");
    }
}
