//! Content-addressed voice cache: `<root>/<cache_key>.wav` (+ `<key>.words.json`
//! when the provider returned word timestamps). A hit never touches the
//! provider, so reruns are offline and byte-identical.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use crate::{cache_key, SpeechWord, Synthesis, TtsModel, VoiceChoice, VoiceError, VoiceProvider};

#[derive(Debug)]
pub struct VoiceCache {
    root: PathBuf,
    calls: Cell<usize>,
    hits: Cell<usize>,
}

/// Default cache root: `$MOTION_VOICE_CACHE`, else `~/.cache/motionengine/voice/`.
pub fn default_root() -> PathBuf {
    root_from(
        std::env::var_os("MOTION_VOICE_CACHE").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// Pure form of [`default_root`].
pub fn root_from(env: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    match env {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => home
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cache")
            .join("motionengine")
            .join("voice"),
    }
}

impl VoiceCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            calls: Cell::new(0),
            hits: Cell::new(0),
        }
    }

    pub fn with_default_root() -> Self {
        Self::new(default_root())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Provider (network/synthesis) calls made through this cache.
    pub fn calls(&self) -> usize {
        self.calls.get()
    }

    /// Cache hits served without calling the provider.
    pub fn hits(&self) -> usize {
        self.hits.get()
    }

    fn wav_path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.wav"))
    }

    fn words_path(&self, key: &str) -> PathBuf {
        self.root.join(format!("{key}.words.json"))
    }

    fn read_entry(&self, key: &str) -> Option<Synthesis> {
        let wav = std::fs::read(self.wav_path(key)).ok()?;
        let words = std::fs::read(self.words_path(key))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<SpeechWord>>(&b).ok());
        Some(Synthesis { wav, words })
    }

    fn write_entry(&self, key: &str, s: &Synthesis) -> Result<(), VoiceError> {
        std::fs::create_dir_all(&self.root)?;
        // Words first: a visible wav implies its words are already in place.
        if let Some(words) = &s.words {
            let json = serde_json::to_vec_pretty(words)
                .map_err(|e| VoiceError::Audio(format!("words json: {e}")))?;
            write_atomic(&self.words_path(key), &json)?;
        }
        write_atomic(&self.wav_path(key), &s.wav)?;
        Ok(())
    }

    /// Cached synthesis, or call `provider` (never when `offline`: then a miss
    /// is [`VoiceError::OfflineMiss`]).
    pub fn get_or_synthesize(
        &self,
        provider: &dyn VoiceProvider,
        text: &str,
        voice: &VoiceChoice,
        model: TtsModel,
        offline: bool,
    ) -> Result<Synthesis, VoiceError> {
        let key = cache_key(model, voice, text);
        if let Some(hit) = self.read_entry(&key) {
            self.hits.set(self.hits.get() + 1);
            return Ok(hit);
        }
        if offline {
            return Err(VoiceError::OfflineMiss(format!(
                "\"{}\" ({}, voice '{}')",
                text,
                model.model_id(),
                voice.voice
            )));
        }
        self.calls.set(self.calls.get() + 1);
        let s = provider.synthesize(text, voice, model)?;
        self.write_entry(&key, &s)?;
        Ok(s)
    }
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::testutil::TempDir;
    use crate::fixture::FixtureProvider;

    struct Counting(FixtureProvider, Cell<usize>);
    impl VoiceProvider for Counting {
        fn id(&self) -> &'static str {
            "fixture"
        }
        fn synthesize(
            &self,
            text: &str,
            voice: &VoiceChoice,
            model: TtsModel,
        ) -> Result<Synthesis, VoiceError> {
            self.1.set(self.1.get() + 1);
            self.0.synthesize(text, voice, model)
        }
    }

    fn voice() -> VoiceChoice {
        VoiceChoice {
            voice: "v".into(),
            say_rate: None,
        }
    }

    #[test]
    fn hit_makes_zero_provider_calls_and_is_identical() {
        let dir = TempDir::new("cache");
        let cache = VoiceCache::new(dir.path());
        let p = Counting(FixtureProvider::new(true), Cell::new(0));
        let a = cache
            .get_or_synthesize(&p, "One two.", &voice(), TtsModel::Say, false)
            .unwrap();
        assert_eq!((p.1.get(), cache.calls(), cache.hits()), (1, 1, 0));

        let b = cache
            .get_or_synthesize(&p, "One two.", &voice(), TtsModel::Say, false)
            .unwrap();
        assert_eq!((p.1.get(), cache.calls(), cache.hits()), (1, 1, 1));
        assert_eq!(a, b);
        assert!(b.words.is_some());

        // A fresh cache object over the same dir (a "rerun") also hits.
        let again = VoiceCache::new(dir.path());
        let c = again
            .get_or_synthesize(&p, "One two.", &voice(), TtsModel::Say, true)
            .unwrap();
        assert_eq!((p.1.get(), again.calls(), again.hits()), (1, 0, 1));
        assert_eq!(a.wav, c.wav);
    }

    #[test]
    fn no_words_file_when_provider_has_none() {
        let dir = TempDir::new("nowords");
        let cache = VoiceCache::new(dir.path());
        let p = FixtureProvider::new(false);
        cache
            .get_or_synthesize(&p, "Hi.", &voice(), TtsModel::Say, false)
            .unwrap();
        let key = cache_key(TtsModel::Say, &voice(), "Hi.");
        assert!(dir.path().join(format!("{key}.wav")).exists());
        assert!(!dir.path().join(format!("{key}.words.json")).exists());
        let hit = cache
            .get_or_synthesize(&p, "Hi.", &voice(), TtsModel::Say, true)
            .unwrap();
        assert!(hit.words.is_none());
    }

    #[test]
    fn offline_miss_errors_without_calling_provider() {
        let dir = TempDir::new("offline");
        let cache = VoiceCache::new(dir.path());
        let p = Counting(FixtureProvider::new(false), Cell::new(0));
        let err = cache
            .get_or_synthesize(&p, "Nope.", &voice(), TtsModel::Say, true)
            .unwrap_err();
        assert!(matches!(err, VoiceError::OfflineMiss(_)));
        assert_eq!(p.1.get(), 0);
        assert_eq!(cache.calls(), 0);
    }

    #[test]
    fn different_voice_is_a_different_entry() {
        let dir = TempDir::new("keys");
        let cache = VoiceCache::new(dir.path());
        let p = FixtureProvider::new(false);
        let other = VoiceChoice {
            voice: "w".into(),
            say_rate: None,
        };
        cache
            .get_or_synthesize(&p, "Hi.", &voice(), TtsModel::Say, false)
            .unwrap();
        cache
            .get_or_synthesize(&p, "Hi.", &other, TtsModel::Say, false)
            .unwrap();
        assert_eq!(cache.calls(), 2);
    }

    #[test]
    fn root_resolution() {
        assert_eq!(
            root_from(Some("/c".into()), Some("/h".into())),
            PathBuf::from("/c")
        );
        assert_eq!(
            root_from(None, Some("/h".into())),
            PathBuf::from("/h/.cache/motionengine/voice")
        );
    }
}
