//! Live attitude, ride height and foil angles for the datalogger's LCD: the real-time
//! data the e-ink displays are too slow for.
//!
//! [`source`] turns CAN frames (live, from a candump log, or a built-in demo) into a
//! [`state::Model`]; [`render::draw`] paints a frame of it; [`fb`] puts that on the
//! Linux framebuffer.

pub mod config;
pub mod fb;
pub mod render;
pub mod source;
pub mod state;
