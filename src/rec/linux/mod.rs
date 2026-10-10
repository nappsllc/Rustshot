//! Linux recording back end (X11): frames from XShm with the pointer from
//! XFixes ([`xshm`]), sound read in-app from PulseAudio sources through
//! parec ([`parec`], listed by [`pulse_list`]) and mixed by the shared
//! mixer, and MP4 through one ffmpeg subprocess fed both ([`ffmpeg`]).
//! Without ffmpeg only GIF is recorded (the built-in OpenH264 fallback
//! comes later).

#[cfg(target_os = "linux")]
pub mod child;
pub mod feed;
pub mod ffmpeg;
pub mod parec;
pub mod pulse_list;
#[cfg(target_os = "linux")]
pub mod xshm;

#[cfg(all(test, target_os = "linux"))]
mod tests;
