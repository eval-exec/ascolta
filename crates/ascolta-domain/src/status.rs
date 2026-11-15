use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlaybackState {
    Playing,
    Paused,
    #[default]
    Stopped,
}

impl PlaybackState {
    pub fn is_playing(self) -> bool {
        matches!(self, Self::Playing)
    }

    pub fn is_stopped(self) -> bool {
        matches!(self, Self::Stopped)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub bits: u8,
    pub channels: u8,
}

impl AudioFormat {
    /// Parses MPD's `sample_rate:bits:channels` format.
    pub fn parse(value: &str) -> Option<Self> {
        let mut parts = value.split(':');
        let sample_rate = parts.next()?.parse().ok()?;
        let bits = parts.next()?.parse().ok()?;
        let channels = parts.next()?.parse().ok()?;
        Some(Self {
            sample_rate,
            bits,
            channels,
        })
    }
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let layout = match self.channels {
            1 => "mono",
            2 => "stereo",
            _ => {
                return write!(
                    f,
                    "{} Hz / {} bit / {} ch",
                    self.sample_rate, self.bits, self.channels
                );
            }
        };
        write!(
            f,
            "{} Hz / {} bit / {}",
            self.sample_rate, self.bits, layout
        )
    }
}

/// The contents of MPD's `status` response.
#[derive(Debug, Clone, Default)]
pub struct MpdStatus {
    pub volume: Option<u8>,
    pub repeat: bool,
    pub random: bool,
    pub single: bool,
    pub consume: bool,
    pub crossfade: Option<u32>,
    pub partition: Option<Arc<str>>,
    pub playlist_version: Option<u32>,
    pub playlist_length: u32,
    pub state: PlaybackState,
    pub song_pos: Option<u32>,
    pub song_id: Option<u32>,
    pub elapsed: Duration,
    pub duration: Option<Duration>,
    pub bitrate: Option<u32>,
    pub audio: Option<AudioFormat>,
    pub mixramp_db: Option<f32>,
    pub mixramp_delay: Option<f32>,
    pub error: Option<String>,
}

impl MpdStatus {
    /// Fraction played in `0.0..=1.0`, if the length is known.
    pub fn progress(&self) -> Option<f64> {
        let total = self.duration?.as_secs_f64();
        if total <= 0.0 {
            return None;
        }
        Some((self.elapsed.as_secs_f64() / total).clamp(0.0, 1.0))
    }

    pub fn elapsed_text(&self) -> String {
        crate::format_duration(self.elapsed)
    }

    pub fn duration_text(&self) -> String {
        self.duration
            .map(crate::format_duration)
            .unwrap_or_else(|| "--:--".into())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MpdOutput {
    pub id: u32,
    pub name: Arc<str>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MpdStats {
    pub artists: u32,
    pub albums: u32,
    pub songs: u32,
    pub uptime: Duration,
    pub playtime: Duration,
    pub db_playtime: Duration,
    pub db_update: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_audio_format() {
        assert_eq!(
            AudioFormat::parse("48000:16:2"),
            Some(AudioFormat {
                sample_rate: 48_000,
                bits: 16,
                channels: 2
            })
        );
        assert_eq!(AudioFormat::parse("garbage"), None);
    }

    #[test]
    fn progress_is_clamped() {
        let mut status = MpdStatus {
            elapsed: Duration::from_secs(30),
            duration: Some(Duration::from_secs(60)),
            ..MpdStatus::default()
        };
        assert_eq!(status.progress(), Some(0.5));
        status.elapsed = Duration::from_secs(120);
        assert_eq!(status.progress(), Some(1.0));
        status.duration = None;
        assert_eq!(status.progress(), None);
    }
}
