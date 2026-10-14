use super::*;

/// A scrollable page seen through a `w`×`h` view: the wheel moves the
/// offset by `per` px a notch (clamped at the end); a grab right after a
/// scroll sees the move half done (smooth scrolling).
struct Sim {
    page: Vec<u8>,
    w: u32,
    ph: u32,
    h: u32,
    off: u32,
    target: u32,
    per: u32,
    /// The wheel does nothing (until the fallback).
    deaf: bool,
    scrolls: u32,
}

impl Sim {
    fn new(w: u32, ph: u32, h: u32, per: u32) -> Arc<Mutex<Sim>> {
        let mut page = vec![255u8; (w * ph * 4) as usize];
        // Text-like rows: a unique dark run per line.
        let mut y = 3;
        let mut k = 1u32;
        while y + 8 < ph {
            for yy in y..y + 8 {
                for x in 4..w - 4 {
                    if (x * 31 + k * 17) % 23 < 9 && (x + yy) % 3 != 0 {
                        let i = ((yy * w + x) * 4) as usize;
                        page[i..i + 3].copy_from_slice(&[(k * 37 % 200) as u8, 30, 30]);
                    }
                }
            }
            y += 13 + k % 5;
            k += 1;
        }
        Arc::new(Mutex::new(Sim { page, w, ph, h, off: 0, target: 0, per, deaf: false, scrolls: 0 }))
    }

    fn view(&mut self) -> PixBuf {
        // Half way there, then there.
        if self.off != self.target {
            self.off = if self.target.abs_diff(self.off) > 1 { (self.off + self.target) / 2 } else { self.target };
        }
        let n = (self.w * 4) as usize;
        let rows = self.page[self.off as usize * n..(self.off + self.h) as usize * n].to_vec();
        PixBuf::from_raw(self.w, self.h, rows).expect("view")
    }
}

struct SimWheel(Arc<Mutex<Sim>>, bool);

impl Wheel for SimWheel {
    fn scroll(&mut self, notches: u32) -> Result<()> {
        let mut s = self.0.lock().unwrap();
        s.scrolls += 1;
        if s.deaf && !self.1 {
            return Ok(());
        }
        s.target = (s.target + notches * s.per).min(s.ph - s.h);
        Ok(())
    }
    fn fallback(&mut self) -> bool {
        self.1 = true;
        true
    }
}

fn fast() -> Pace {
    Pace { notches: 2, poll: Duration::from_millis(1), settle: Duration::from_millis(50), min_step: Duration::ZERO }
}

fn go(sim: &Arc<Mutex<Sim>>, ctl: &Ctl) -> (PixBuf, End) {
    let s2 = sim.clone();
    let mut grab = move || Ok(s2.lock().unwrap().view());
    let mut wheel = SimWheel(sim.clone(), false);
    run(&mut grab, &mut wheel, ctl, fast(), &|| false).expect("run")
}

#[test]
fn a_run_stitches_the_whole_page_and_ends_at_the_bottom() {
    let sim = Sim::new(120, 900, 100, 20);
    let ctl = Ctl::default();
    let (pic, end) = go(&sim, &ctl);
    assert_eq!(end, End::Bottom);
    let s = sim.lock().unwrap();
    assert_eq!(pic.dimensions(), (120, 900));
    assert_eq!(pic.as_raw()[..], s.page[..], "the page, exactly");
    assert_eq!(ctl.progress().height, 900);
}

#[test]
fn the_wheel_falls_back_when_it_does_nothing() {
    let sim = Sim::new(120, 500, 100, 20);
    sim.lock().unwrap().deaf = true;
    let (pic, end) = go(&sim, &Ctl::default());
    assert_eq!(end, End::Bottom);
    assert_eq!(pic.height(), 500, "scrolled after the fallback");
}

#[test]
fn stop_keeps_what_was_stitched() {
    let sim = Sim::new(120, 900, 100, 20);
    let ctl = Ctl::default();
    ctl.stop();
    let (pic, end) = go(&sim, &ctl);
    assert_eq!(end, End::Stopped);
    assert_eq!(pic.dimensions(), (120, 100), "the first frame");
}

#[test]
fn steps_aim_for_half_the_frame() {
    assert_eq!(adapt(3, 60, 400), 10, "20 px a notch: 200 px is 10");
    assert_eq!(adapt(10, 600, 400), 3, "60 px a notch");
    assert_eq!(adapt(2, 0, 400), 4, "nothing moved: double");
    assert_eq!(adapt(20, 0, 400), MAX_NOTCHES);
    assert_eq!(adapt(1, 2000, 400), 1, "at least one");
}

#[test]
fn ends_say_why() {
    assert_eq!(End::Lost.note(), Some(text::LOST.get()));
    assert_eq!(End::Cap.note(), Some(text::CAP.get()));
    assert_eq!(End::Bottom.note(), None);
    assert_eq!(End::Stopped.note(), None);
}
