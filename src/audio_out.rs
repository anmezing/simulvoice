use anyhow::Result;
use rodio::{OutputStream, Sink};
use std::sync::Arc;

/// Shared playback controller for realtime engines.
pub struct AudioOutputController {
    _stream: OutputStream,
    sink: Arc<Sink>,
}

impl AudioOutputController {
    pub fn open(device: Option<&str>) -> Result<Self> {
        let (stream, handle) = crate::sink::open_output(device)?;
        let sink = Arc::new(Sink::try_new(&handle)?);
        Ok(Self {
            _stream: stream,
            sink,
        })
    }

    pub fn sink(&self) -> Arc<Sink> {
        self.sink.clone()
    }

    pub fn stop(&self) {
        self.sink.stop();
        self.sink.play();
    }

    pub fn clear(&self) {
        self.sink.stop();
        self.sink.play();
    }
}
