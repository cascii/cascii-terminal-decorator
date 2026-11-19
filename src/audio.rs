use crate::input::Input;
use anyhow::{Context, Result};
use cascii_core_view::AnimationController;
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use std::{fs::File, path::PathBuf, time::Duration};

pub struct Audio {
    stream: OutputStream,
    sink: Sink,
    path: PathBuf,
    _download: Option<tempfile::TempDir>,
    source_fps: u32,
    volume: f32,
    muted: bool,
}

impl Audio {
    pub fn open(input: Input, fps: u32) -> Result<Self> {
        let (path, download) = match input {
            Input::Path(path) => (path, None),
            remote => {
                let dir = tempfile::tempdir()?;
                let path = dir.path().join("audio.mp3");
                remote.copy_to(&path)?;
                (path, Some(dir))
            }
        };
        let mut stream = OutputStreamBuilder::open_default_stream().context("opening audio output")?;
        stream.log_on_drop(false);
        let sink = Sink::connect_new(stream.mixer());
        Ok(Self {stream, sink, path, _download: download, source_fps: fps.max(1), volume: 1.0, muted: false})
    }

    // Seek on the source timeline before applying the playback-rate multiplier.
    // Reopening also handles a track that ended before the animation looped.
    pub fn sync(&mut self, controller: &AnimationController) -> Result<()> {
        self.sink.stop();
        if !controller.is_playing() { return Ok(()); }
        let mut source = Decoder::try_from(File::open(&self.path)?)?;
        let position = Duration::from_secs_f64(controller.current_frame() as f64 / f64::from(self.source_fps));
        if source.total_duration().is_some_and(|end| position >= end) { return Ok(()); }
        if !position.is_zero() { source.try_seek(position).map_err(|e| anyhow::anyhow!("seeking soundtrack: {e}"))?; }
        self.sink = Sink::connect_new(self.stream.mixer());
        self.sink.set_speed(controller.fps() as f32 / self.source_fps as f32);
        self.update_volume();
        self.sink.append(source);
        Ok(())
    }

    fn update_volume(&self) {
        self.sink.set_volume(if self.muted {0.0} else {self.volume});
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.update_volume();
    }

    pub fn adjust_volume(&mut self, amount: f32) {
        self.volume = (self.volume + amount).clamp(0.0, 1.0);
        self.update_volume();
    }

    pub fn level(&self) -> String {
        if self.muted {"muted".into()} else {format!("{:.0}%", self.volume * 100.0)}
    }
}
