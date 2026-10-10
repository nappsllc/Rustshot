//! Linux recording back end (X11): frames from XShm with the pointer from
//! XFixes ([`xshm`]), MP4 through an ffmpeg subprocess that also records
//! the sound ([`ffmpeg`]), and the PulseAudio sources it can record
//! ([`pulse_list`]). Without ffmpeg only GIF is recorded (the built-in
//! OpenH264 fallback comes later).

pub mod ffmpeg;
pub mod pulse_list;
#[cfg(target_os = "linux")]
pub mod xshm;

#[cfg(all(test, target_os = "linux"))]
mod tests;
