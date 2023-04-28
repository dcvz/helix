pub use arie;
mod extensions;
mod graphics;
pub mod gui;
#[cfg(feature = "network")]
pub mod network;
#[cfg(feature = "speech")]
pub mod speech;
mod utils;

// MARK: - C API

#[no_mangle]
pub extern "C" fn SpeechFeatureEnabled() -> bool {
    #[cfg(feature = "speech")]
    return true;
    #[cfg(not(feature = "speech"))]
    return false;
}

#[no_mangle]
pub extern "C" fn NetworkFeatureEnabled() -> bool {
    #[cfg(feature = "network")]
    return true;
    #[cfg(not(feature = "network"))]
    return false;
}
