# Voice Mode

> **Status:** ✅ Complete
> **Category:** Core Features

---

## 1. Overview

Voice mode enables Talon to receive speech input and deliver spoken output.
It's primarily useful for the Telegram gateway (voice messages are common)
and optional for the CLI.

Pipeline:
```
Voice Input (OGG/MP3) → Transcription (Whisper/STT API) → Text → Agent
Agent Response → TTS (edge-tts / OpenAI TTS) → Audio File → Delivery
```

---

## 2. Speech-to-Text (STT)

### Option A: OpenAI Whisper API
```rust
pub struct WhisperSttProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,   // "whisper-1"
}

impl SttProvider for WhisperSttProvider {
    async fn transcribe(&self, audio_path: &Path) -> Result<String, SttError> {
        let file_bytes = tokio::fs::read(audio_path).await?;
        let file_part = reqwest::multipart::Part::bytes(file_bytes)
            .file_name(audio_path.file_name().unwrap().to_str().unwrap().to_string())
            .mime_str("audio/ogg")?;

        let form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("language", "en")   // or auto-detect
            .part("file", file_part);

        let resp: TranscriptionResponse = self.client
            .post("https://api.openai.com/v1/audio/transcriptions")
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send().await?
            .json().await?;

        Ok(resp.text)
    }
}
```

### Option B: Local Whisper (whisper-rs)
For fully offline operation:
```toml
[dependencies]
whisper-rs = { version = "0.11", features = ["cuda"] }  # GPU optional
```

```rust
pub struct LocalWhisperProvider {
    ctx: WhisperContext,
}

impl SttProvider for LocalWhisperProvider {
    async fn transcribe(&self, audio_path: &Path) -> Result<String, SttError> {
        let audio = load_audio_as_f32(audio_path)?;
        let mut state = self.ctx.create_state()?;
        let params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        state.full(params, &audio)?;
        let n = state.full_n_segments()?;
        let text = (0..n).map(|i| state.full_get_segment_text(i).unwrap_or_default())
            .collect::<Vec<_>>().join(" ");
        Ok(text)
    }
}
```

---

## 3. Text-to-Speech (TTS)

### Option A: edge-tts (free, Microsoft Neural TTS)
```bash
edge-tts --voice en-US-AriaNeural --text "Hello, I'm Talon" --write-media output.mp3
```

```rust
pub struct EdgeTtsProvider {
    voice: String,   // e.g. "en-US-AriaNeural"
}

impl TtsProvider for EdgeTtsProvider {
    async fn synthesize(&self, text: &str, output: &Path) -> Result<(), TtsError> {
        let status = tokio::process::Command::new("edge-tts")
            .args(["--voice", &self.voice, "--text", text, "--write-media"])
            .arg(output)
            .status().await?;

        if !status.success() {
            return Err(TtsError::EdgeTtsFailed);
        }
        Ok(())
    }
}
```

### Option B: OpenAI TTS API
```rust
pub struct OpenAiTtsProvider {
    client: reqwest::Client,
    api_key: String,
    voice: String,   // alloy | echo | fable | onyx | nova | shimmer
}

impl TtsProvider for OpenAiTtsProvider {
    async fn synthesize(&self, text: &str, output: &Path) -> Result<(), TtsError> {
        let bytes = self.client
            .post("https://api.openai.com/v1/audio/speech")
            .bearer_auth(&self.api_key)
            .json(&json!({
                "model": "tts-1",
                "voice": self.voice,
                "input": text,
                "response_format": "mp3"
            }))
            .send().await?
            .bytes().await?;

        tokio::fs::write(output, bytes).await?;
        Ok(())
    }
}
```

---

## 4. Telegram Voice Integration

Telegram sends voice messages as OGG Opus files. Talon's Telegram gateway
handles them transparently:

```rust
// In TelegramGateway::listen():
Update::filter_message().endpoint(|bot: Bot, msg: Message, tx| async move {
    let content = if let Some(voice) = msg.voice() {
        // Download voice message
        let file = bot.get_file(&voice.file.id).await?;
        let path = cache_dir.join(format!("{}.ogg", voice.file.unique_id));
        bot.download_file(&file.path, &mut tokio::fs::File::create(&path).await?).await?;

        // Transcribe
        let text = stt_provider.transcribe(&path).await?;
        tracing::debug!("Voice transcribed: {text}");

        InputContent::Voice {
            file_path: path,
            transcription: text,
            duration_secs: voice.duration,
        }
    } else if let Some(text) = msg.text() {
        InputContent::Text(text.to_string())
    } else {
        return respond(());
    };

    tx.send(AgentInput { content, .. }).await.ok();
    respond(())
}),
```

---

## 5. TTS Output

Talon speaks its response when configured for voice output:

```toml
[tts]
enabled = true
provider = "edge"   # or "openai"
voice = "en-US-AriaNeural"

# Only speak final responses (not tool outputs)
speak_final_only = true
# Max chars to speak (truncate for long responses)
max_chars = 10000
```

```rust
pub async fn maybe_speak(
    tts: Option<&dyn TtsProvider>,
    config: &TtsConfig,
    response: &str,
    deliver: &dyn Fn(DeliveryEvent),
) -> Result<(), TtsError> {
    let Some(tts) = tts else { return Ok(()); };
    if !config.enabled { return Ok(()); }

    let text = if response.len() > config.max_chars {
        &response[..config.max_chars]
    } else {
        response
    };

    let path = cache_dir.join(format!("{}.mp3", Uuid::new_v4()));
    tts.synthesize(text, &path).await?;

    deliver(DeliveryEvent::MediaFile {
        path,
        caption: None,
    });
    Ok(())
}
```
---

## Related Documents

### Depends On
- [Gateway Architecture](../02_Architecture/18_Gateway_MultiChannel_Architecture.md)

### See Also
- [Streaming & Realtime Output](31a_Streaming_And_Realtime_Output.md)
- [Telegram Integration](../05_API_Bindings/45_Telegram_Integration.md)

