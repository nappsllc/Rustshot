"""Attribute .text/.rdata bytes of an MSVC /MAP (or an lld/GNU ld -Map) to crate/module.

A symbol's size is the distance to the next symbol (MSVC) or its input
section's size (ELF). Generic code counts under the type it is about
(`drop_in_place<T>`, `<T as Trait>::f`: T). Anonymous rdata is classified by
peeking at the exe: strings, panic locations (by the source path they
point to), the rest "anon".

Windows (keeps .cargo/config.toml's rustflags, which RUSTFLAGS would replace):
  cargo rustc --release --bin rustshot -- -C link-arg=/MAP:<dir>\\rustshot.map
  python tools/size-map.py <dir>\\rustshot.map <target>\\release\\rustshot.exe [top]
Linux:
  RUSTFLAGS="-C link-arg=-Wl,-Map=<dir>/rustshot.map" cargo build --release
  python tools/size-map.py <dir>/rustshot.map - [top]
ANON=1 lists the biggest unclassified anonymous rdata items.
"""
import re, sys, struct, collections, os
anon_list = []

ESC = {'$LT$': '<', '$GT$': '>', '$u20$': ' ', '$C$': ',', '$RF$': '&', '$BP$': '*',
       '$LP$': '(', '$RP$': ')', '$u5b$': '[', '$u5d$': ']', '$u7b$': '{', '$u7d$': '}',
       '$u3b$': ';', '$u27$': "'", '$u2b$': '+', '$u22$': '"', '$SP$': '@', '$u7e$': '~',
       '$u21$': '!', '$u3d$': '='}


def demangle(s):
    if s.startswith('__ZN'):
        s = s[1:]
    if not s.startswith('_ZN'):
        return None
    i, parts = 3, []
    while i < len(s) and s[i] != 'E':
        m = re.match(r'\d+', s[i:])
        if not m:
            break
        n = int(m.group()); i += len(m.group())
        parts.append(s[i:i + n]); i += n
    if parts and re.fullmatch(r'h[0-9a-f]{16}', parts[-1]):
        parts.pop()
    out = '::'.join(parts)
    for k, v in ESC.items():
        out = out.replace(k, v)
    out = out.replace('..', '::')
    if out.startswith('_'):
        out = out[1:] if out[1:2] in '<&' else out
    return out


def v0(s):
    if not s.startswith('_R'):
        return None
    m = re.search(r'C(?:s[0-9a-zA-Z]*_)?(\d+)([A-Za-z_][A-Za-z0-9_]*)', s)
    if not m:
        return None
    n = int(m.group(1)); crate = m.group(2)[:n]
    rest = s[m.start(2) + n:]
    segs = [crate]
    for mm in re.finditer(r'(?<![0-9a-zA-Z])(\d+)([A-Za-z_][A-Za-z0-9_]*)', rest):
        k = int(mm.group(1)); segs.append(mm.group(2)[:k])
        if len(segs) > 3:
            break
    return '::'.join(segs)


def owner(d):
    """The path a symbol is 'about': drop_in_place<T> -> T, <T as Tr>::f -> T."""
    for pre in ('core::ptr::drop_in_place<',):
        while d.startswith(pre):
            d = d[len(pre):]
    while d and d[0] in '<&*[(':
        d = d[1:]
        for kw in ('mut ', 'const ', 'dyn '):
            if d.startswith(kw):
                d = d[len(kw):]
    path = re.split(r'[<> ,\[\]();]', d, maxsplit=1)[0]
    segs = [p for p in path.split('::') if p]
    return segs


def group(segs, lib):
    if not segs:
        return (lib or '?', '')
    c = segs[0]
    if c in ('rustshot',) and len(segs) > 1:
        mod = segs[1]
        if len(segs) > 2 and mod in ('cam', 'rec', 'editor', 'edit_ui', 'objects', 'ui', 'settings_ui', 'history', 'rec_ui', 'imgedit', 'ocr', 'share', 's3', 'keystore', 'uploads', 'dbus', 'sni', 'launcher', 'history_ui') and not segs[2][0].isupper() and segs[2] not in ('new',):
            mod += '::' + segs[2]
        return (c, mod)
    return (c, segs[1] if len(segs) > 1 else '')


def pe_sections(exe):
    b = open(exe, 'rb').read()
    pe = struct.unpack_from('<I', b, 0x3c)[0]
    nsec = struct.unpack_from('<H', b, pe + 6)[0]
    opt = struct.unpack_from('<H', b, pe + 20)[0]
    off = pe + 24 + opt
    secs = []
    for k in range(nsec):
        name, vsize, va, rsize, roff = struct.unpack_from('<8sIIII', b, off + 40 * k)
        secs.append((va, vsize, roff))
    return b, secs


def read_rva(b, secs, rva, n):
    for va, vs, ro in secs:
        if va <= rva < va + vs:
            return b[ro + rva - va: ro + rva - va + n]
    return b''


def guess_anon(data):
    t = data.decode('latin1')
    m = re.search(r'registry[\\/]src[\\/][^\\/]+[\\/]([A-Za-z0-9_\-]+?)-\d+\.\d+', t)
    if m:
        return ('anon-src', m.group(1))
    m = re.search(r'(?:library|rustc[\\/][0-9a-f]+[\\/]library)[\\/](\w+)', t)
    if m:
        return ('anon-src', m.group(1))
    m = re.search(r'src[\\/]([\w\\/]+)\.rs', t)
    if m:
        return ('anon-src', 'rustshot::' + m.group(1).replace('\\', '/').split('/')[0])
    return None


def parse_msvc(path):
    syms = []
    sect = {}
    for line in open(path, encoding='latin1'):
        m = re.match(r' (000[12]):([0-9a-f]{8}) ([0-9a-f]{8})H (\S+)\s+(CODE|DATA)', line)
        if m:
            sect.setdefault(m.group(1), []).append((int(m.group(2), 16), int(m.group(3), 16), m.group(4)))
            continue
        m = re.match(r' (000[12]):([0-9a-f]{8})\s+(\S+)\s+([0-9a-f]{16})\s+(?:f\s+)?(?:i\s+)?(\S.*)?$', line)
        if m:
            syms.append((m.group(1), int(m.group(2), 16), m.group(3), int(m.group(4), 16), (m.group(5) or '').strip()))
    return syms, sect


def elf_items(mp):
    # lld -Map: VMA LMA Size Align Out In Symbol; input-section lines are 'file:(.text.sym)'
    for line in open(mp, encoding='latin1'):
        m = re.match(r'\s+[0-9a-f]+\s+[0-9a-f]+\s+([0-9a-f]+)\s+\d+\s{9}(\S.*?):\((\.(?:text|rodata|data\.rel\.ro)[^)]*)\)\s*$', line)
        if m:
            size = int(m.group(1), 16)
            sec = m.group(3)
            kind = 'text' if sec.startswith('.text') else 'rdata'
            sym = re.sub(r'^\.(text|rodata|data\.rel\.ro)(\.unlikely|\.hot|\.cst\d+|\.str[\d.]*)?\.?', '', sec)
            yield size, kind, sym, m.group(2)


def main_elf(mp, top):
    totals = collections.Counter(); big = []
    for size, kind, sym, f in elf_items(mp):
        d = demangle(sym) or v0(sym)
        if d:
            g = group(owner(d), '')
        elif sym == '' or sym.startswith('.L') or sym.startswith('anon') :
            g = ('anon/merged', '')
        else:
            lib = f.rsplit('/', 1)[-1].split('(')[0].split('.')[0][:20]
            g = ('<native>', lib)
        totals[(g, kind)] += size
        big.append((size, kind, d or sym or f))
    report(totals, big, collections.Counter(), top)


def main():
    mp = sys.argv[1]
    top = int(sys.argv[3]) if len(sys.argv) > 3 else 40
    if open(mp, encoding='latin1').readline().strip().startswith('VMA'):
        return main_elf(mp, top)
    exe = sys.argv[2]
    syms, sect = parse_msvc(mp)
    b, secs = pe_sections(exe)
    base = 0x140000000
    totals = collections.Counter()
    big = []
    fam = collections.Counter()
    for sec in ('0001', '0002'):
        ends = sum(1 for _ in [])
        ss = sorted({(s[1], s[2], s[3], s[4]) for s in syms if s[0] == sec})
        sec_end = max(o + l for o, l, n in sect[sec])
        # dedupe identical addresses (ICF): keep first
        seen = {}
        for off, name, rva, lib in ss:
            seen.setdefault(off, (name, rva, lib))
        offs = sorted(seen)
        for k, off in enumerate(offs):
            nxt = offs[k + 1] if k + 1 < len(offs) else sec_end
            # don't run across a section piece boundary into e.g. .xdata/.pdata
            for o, l, n in sect[sec]:
                if o <= off < o + l:
                    nxt = min(nxt, o + l)
                    piece = n
            size = nxt - off
            name, rva, lib = seen[off]
            libn = lib.split(':')[0] if ':' in lib else lib.split('.')[0]
            if off >= sec_end:
                continue
            d = demangle(name) or v0(name)
            kind = 'text' if sec == '0001' else 'rdata'
            if sec == '0002' and piece not in ('.rdata',):
                g = ('<' + piece + '>', '')
            elif d:
                g = group(owner(d), libn)
                fam[re.sub(r'<.*', '<..>', d)] += size
            elif name.startswith('anon.') or name.startswith('.L') or name.startswith('__unnamed') or name.startswith('??_C') or name.startswith('alloc_'):
                data = read_rva(b, secs, rva - base, min(size, 512))
                gu = guess_anon(data)
                if not gu and len(data) >= 8:
                    ptr = struct.unpack_from('<Q', data)[0]
                    if base < ptr < base + 0x1000000:
                        t = guess_anon(read_rva(b, secs, ptr - base, 200))
                        if t:
                            gu = ('anon-ptr', t[1])
                if not gu and size < 0x10000 and data and sum(32 <= c < 127 or c in (9, 10, 13) for c in data) > 0.9 * len(data):
                    gu = ('anon-str', libn)
                g = gu if gu else ('anon', libn)
                if not gu and os.environ.get('ANON'):
                    anon_list.append((size, name, read_rva(b, secs, rva - base, 40)))
            else:
                g = ('<native>', libn)
            totals[(g, kind)] += size
            big.append((size, kind, d or name))
    for sz, n, dd in sorted(anon_list, reverse=True)[:40]:
        print('ANON', sz, n, dd)
    report(totals, big, fam, top)


def report(totals, big, fam, top):
    agg = collections.Counter()
    for (g, kind), v in totals.items():
        agg[g] += v
    print(f"{'crate':<22} {'module':<28} {'text':>9} {'rdata':>9} {'total':>9}")
    for g, v in agg.most_common(top):
        print(f"{g[0][:22]:<22} {g[1][:28]:<28} {totals[(g,'text')]:>9} {totals[(g,'rdata')]:>9} {v:>9}")
    crates = collections.Counter()
    for g, v in agg.items():
        crates[g[0]] += v
    print('\n-- by crate --')
    for c, v in crates.most_common(25):
        print(f'{c:<30} {v:>9}')
    print('\n-- biggest symbols --')
    for s, k, n in sorted(big, reverse=True)[:30]:
        print(f'{s:>8} {k:<5} {n[:150]}')
    print('\n-- biggest generic families (demangled, generics erased) --')
    for n, v in fam.most_common(30):
        print(f'{v:>8} {n[:150]}')
    t = sum(v for (g, k), v in totals.items() if k == 'text')
    r = sum(v for (g, k), v in totals.items() if k == 'rdata')
    print(f'\ntext {t}  rdata-all {r}')


main()
