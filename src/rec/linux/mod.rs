//! Linux recording back end (X11): frames from XShm with the pointer from
//! XFixes ([`xshm`]); sound read in-app from PulseAudio sources through
//! libpulse-simple ([`pulse`]), or parec when the library is missing
//! ([`parec`]), listed by [`pulse_list`] and mixed by the shared mixer.
//! MP4 through one ffmpeg subprocess fed both ([`ffmpeg`]), or, without
//! ffmpeg, through Cisco's OpenH264 (downloaded on request, [`openh264`])
//! and the system's libopus ([`opus`]) in Rustshot's own muxer
//! ([`native`], [`crate::rec::mp4`]); [`codec_dialog`] asks which the
//! first time neither is there.

#[cfg(target_os = "linux")]
pub mod child;
pub mod codec_dialog;
#[cfg(target_os = "linux")]
pub mod dl;
pub mod feed;
pub mod ffmpeg;
pub mod native;
pub mod openh264;
pub mod opus;
pub mod parec;
#[cfg(target_os = "linux")]
pub mod pipewire;
pub mod pipewire_pod;
pub mod portal;
pub mod pulse;
pub mod pulse_list;
pub mod v4l2;
#[cfg(target_os = "linux")]
pub mod xshm;

#[cfg(all(test, target_os = "linux"))]
mod tests;
#[cfg(all(test, target_os = "linux"))]
mod tests_wayland;
