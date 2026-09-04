//! Moonlight Scarlet application entry point.

#![cfg_attr(target_os = "scarlet", feature(portable_simd))]

#[cfg(target_os = "scarlet")]
use std::num::NonZeroU32;

mod audio;
mod input;
mod licenses;
mod stream;
mod ui;
mod video;

#[cfg(target_os = "scarlet")]
const CUSTOM_RANDOM_ERROR: u32 = getrandom::Error::CUSTOM_START + 1;

#[cfg(target_os = "scarlet")]
getrandom::register_custom_getrandom!(scarlet_getrandom);

#[cfg(target_os = "scarlet")]
fn scarlet_getrandom(destination: &mut [u8]) -> Result<(), getrandom::Error> {
    let mut offset = 0;
    while offset < destination.len() {
        let result = scarlet_sys::syscall3(
            scarlet_sys::Syscall::GetRandom,
            destination[offset..].as_mut_ptr() as usize,
            destination.len() - offset,
            scarlet_sys::GET_RANDOM_FLAG_REQUIRE_ENTROPY,
        );
        if result == usize::MAX || result == 0 || result > destination.len() - offset {
            let code = NonZeroU32::new(CUSTOM_RANDOM_ERROR)
                .expect("custom getrandom error code must be non-zero");
            return Err(getrandom::Error::from(code));
        }
        offset += result;
    }
    Ok(())
}

fn main() {
    if let Err(error) = ui::run() {
        eprintln!("moonlight: {error}");
    }
}
