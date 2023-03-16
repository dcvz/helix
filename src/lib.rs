pub mod audio;
#[cfg(feature = "cpp")]
mod macros;
#[cfg(feature = "network")]
pub mod network;
#[cfg(feature = "speech")]
pub mod speech;
pub mod window;

#[cfg(feature = "cpp")]
use lazy_static::lazy_static;
#[cfg(feature = "cpp")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "cpp")]
lazy_static! {
    static ref AUDIO_PLAYER: Arc<Mutex<audio::AudioPlayer>> =
        Arc::new(Mutex::new(audio::AudioPlayer::new()));
}

#[cfg(feature = "cpp")]
#[cfg(feature = "speech")]
lazy_static! {
    static ref SPEECH_SYNTHESIZER: Arc<Mutex<speech::SpeechSynthesizer>> =
        Arc::new(Mutex::new(speech::SpeechSynthesizer::new()));
}

#[cfg(feature = "cpp")]
#[cfg(feature = "network")]
lazy_static! {
    static ref TCP_STREAM: Arc<Mutex<network::TCPStream>> =
        Arc::new(Mutex::new(network::TCPStream::new()));
}

// MARK: - C API

#[cfg(feature = "cpp")]
#[no_mangle]
pub extern "C" fn HLXSpeechFeatureEnabled() -> bool {
    #[cfg(feature = "speech")]
    return true;
    #[cfg(not(feature = "speech"))]
    return false;
}

#[cfg(feature = "cpp")]
#[no_mangle]
pub extern "C" fn HLXNetworkFeatureEnabled() -> bool {
    #[cfg(feature = "network")]
    return true;
    #[cfg(not(feature = "network"))]
    return false;
}
