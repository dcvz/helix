use crate::HELIX;
use tts::*;
use std::ffi::CStr;
use std::str;

#[repr(C)]
pub enum SpeechSynthesizerGender {
    Male,
    Female,
    Neutral,
}

impl SpeechSynthesizerGender {
    fn to_lib(&self) -> Option<Gender> {
        match self {
            SpeechSynthesizerGender::Male => Option::Some(Gender::Male),
            SpeechSynthesizerGender::Female => Option::Some(Gender::Female),
            SpeechSynthesizerGender::Neutral => Option::None,
        }
    }
}

pub struct SpeechSynthesizer {
    config: Option<Config>,
}

pub struct Config {
    backend: Tts,
    language: LanguageTag<String>,
    gender: SpeechSynthesizerGender,
}

impl SpeechSynthesizer {
    pub fn new() -> Self {
        SpeechSynthesizer {
            config: Option::None,
        }
    }

    pub fn init(&mut self) {
        let backend = Tts::default();

        match backend {
            Ok(backend) => {
                self.config = Some(Config {
                    backend,
                    language: LanguageTag::parse("en-US").unwrap().into(),
                    gender: SpeechSynthesizerGender::Neutral,
                })
            }
            Err(e) => {
                println!("[Speech Synthesizer] Error initializing speech synthesizer: {}", e);
            }
        }
    }

    pub fn deinit(&mut self) {
        if let Some(_) = &self.config {
            self.config = Option::None;
        }
    }

    pub fn set_volume(&mut self, volume: f32) {
        if let Some(config) = self.config.as_mut() {
            _ = config.backend.set_volume(volume);
        }
    }

    pub fn set_language(&mut self, language: &str) {
        if let Some(config) = self.config.as_mut() {
            let language = LanguageTag::parse(language);

            if let Ok(language) = language {
                config.language = language.into();
                self.set_voice();
            } else {
                println!("[Speech Synthesizer] Error parsing language: {}", language.unwrap_err());
            }
        }
    }

    pub fn set_gender(&mut self, gender: SpeechSynthesizerGender) {
        if let Some(config) = self.config.as_mut() {
            config.gender = gender;
            self.set_voice();
        }
    }

    pub fn speak(&mut self, text: &str, interrupt: bool) {
        if let Some(config) = self.config.as_mut() {
            _ = config.backend.speak(text, interrupt);
        }
    }

    fn set_voice(&mut self) {
        if let Some(config) = self.config.as_mut() {
            // if voices are found, let's try to find one for the given gender
            if let Ok(voices) = config.backend.voices() {
                let voice = &voices
                    .iter()
                    .find(|v| v.gender() == config.gender.to_lib() && v.language() == config.language)
                    .unwrap();

                _ = config.backend.set_voice(voice);
            }
        }
    }
}

// MARK: - C API

#[no_mangle]
pub extern "C" fn HLXSpeechSynthesizerInit() {
    HELIX.lock().unwrap().speech_synthesizer.init();
}

#[no_mangle]
pub extern "C" fn HLXSpeechSynthesizerDeinit() {
    HELIX.lock().unwrap().speech_synthesizer.deinit();
}

#[no_mangle]
pub extern "C" fn HLXSpeechSynthesizerSetVolume(volume: f32) {
    HELIX.lock().unwrap().speech_synthesizer.set_volume(volume);
}

#[no_mangle]
pub extern "C" fn HLXSpeechSynthesizerSetLanguage(language_raw: *const i8) {
    let language_str: &CStr = unsafe { CStr::from_ptr(language_raw) };
    let language: &str = str::from_utf8(language_str.to_bytes()).unwrap();

    HELIX.lock().unwrap().speech_synthesizer.set_language(language);
}

#[no_mangle]
pub extern "C" fn HLXSpeechSynthesizerSetGender(gender: SpeechSynthesizerGender) {
    HELIX.lock().unwrap().speech_synthesizer.set_gender(gender);
}

#[no_mangle]
pub extern "C" fn HLXSpeechSynthesizerSpeak(text_raw: *const i8, interrupt: u8) {
    let text_str: &CStr = unsafe { CStr::from_ptr(text_raw) };
    let text: &str = str::from_utf8(text_str.to_bytes()).unwrap();

    HELIX.lock().unwrap().speech_synthesizer.speak(text, interrupt != 0);
}