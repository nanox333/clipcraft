//! The software encoder on a thread of its own, so the next picture can be read back from the
//! screen and converted while the encoder is still working on the last one. Without this the two
//! costs add up inside one frame; with it only the slower of them counts.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;

use filmcraft_h264enc::{Encoder, Packet, YuvFrame};

use crate::RecordError;
use crate::convert::I420;

/// How many pictures may be in flight: one being encoded, one being filled.
const BUFFERS: usize = 2;

enum Job {
    Encode(I420, i64),
    Flush,
}

enum Done {
    Frame { packets: Vec<Packet>, picture: I420 },
    Failed,
    Flushed(Vec<Packet>),
}

/// What a finished job hands back, for the caller to store.
pub(crate) enum Delivery {
    /// The packets of one encoded picture.
    Frame(Vec<Packet>),
    /// The packets left in the encoder when the stream ended.
    Tail(Vec<Packet>),
}

pub(crate) struct SoftwareEncoder {
    jobs: Option<Sender<Job>>,
    done: Receiver<Done>,
    handle: Option<JoinHandle<()>>,
    pool: Vec<I420>,
    outstanding: usize,
    avcc: Vec<u8>,
}

impl SoftwareEncoder {
    /// Moves `encoder` onto its own thread. `width` and `height` size the picture buffers.
    pub(crate) fn start(encoder: Encoder, width: u32, height: u32) -> Result<Self, RecordError> {
        let avcc = encoder.avcc();
        let mut pool = Vec::with_capacity(BUFFERS);
        for _ in 0..BUFFERS {
            pool.push(I420::black(width, height)?);
        }
        let (jobs, job_receiver) = mpsc::channel::<Job>();
        let (done_sender, done) = mpsc::channel::<Done>();
        let handle = std::thread::Builder::new()
            .name("clipcraft-encoder".to_owned())
            .spawn(move || worker(encoder, &job_receiver, &done_sender))
            .map_err(|_| RecordError::EncoderFailed)?;
        Ok(Self {
            jobs: Some(jobs),
            done,
            handle: Some(handle),
            pool,
            outstanding: 0,
            avcc,
        })
    }

    /// The decoder configuration for the clip's header.
    pub(crate) fn avcc(&self) -> Vec<u8> {
        self.avcc.clone()
    }

    fn take(
        &mut self,
        done: Done,
        deliver: &mut impl FnMut(Delivery),
    ) -> Result<bool, RecordError> {
        match done {
            Done::Frame { packets, picture } => {
                self.pool.push(picture);
                self.outstanding = self.outstanding.saturating_sub(1);
                deliver(Delivery::Frame(packets));
                Ok(false)
            }
            Done::Flushed(packets) => {
                deliver(Delivery::Tail(packets));
                Ok(true)
            }
            Done::Failed => {
                self.outstanding = self.outstanding.saturating_sub(1);
                Err(RecordError::EncoderFailed)
            }
        }
    }

    /// Hands out whatever the encoder has finished, without waiting.
    pub(crate) fn poll(&mut self, deliver: &mut impl FnMut(Delivery)) -> Result<(), RecordError> {
        while let Ok(done) = self.done.try_recv() {
            self.take(done, deliver)?;
        }
        Ok(())
    }

    /// A free picture buffer. Waits for the encoder when both are still in use, which is how a
    /// slow encoder holds the rest of the pipeline back instead of letting pictures pile up.
    pub(crate) fn acquire(
        &mut self,
        deliver: &mut impl FnMut(Delivery),
    ) -> Result<I420, RecordError> {
        self.poll(deliver)?;
        while self.pool.is_empty() {
            let done = self.done.recv().map_err(|_| RecordError::EncoderFailed)?;
            self.take(done, deliver)?;
        }
        self.pool.pop().ok_or(RecordError::EncoderFailed)
    }

    /// Queues a filled picture for encoding at time `pts`.
    pub(crate) fn submit(&mut self, picture: I420, pts: i64) -> Result<(), RecordError> {
        self.jobs
            .as_ref()
            .ok_or(RecordError::EncoderFailed)?
            .send(Job::Encode(picture, pts))
            .map_err(|_| RecordError::EncoderFailed)?;
        self.outstanding += 1;
        Ok(())
    }

    /// Waits for everything queued, then drains the encoder.
    pub(crate) fn flush(&mut self, deliver: &mut impl FnMut(Delivery)) -> Result<(), RecordError> {
        self.jobs
            .as_ref()
            .ok_or(RecordError::EncoderFailed)?
            .send(Job::Flush)
            .map_err(|_| RecordError::EncoderFailed)?;
        loop {
            let done = self.done.recv().map_err(|_| RecordError::EncoderFailed)?;
            if self.take(done, deliver)? {
                return Ok(());
            }
        }
    }
}

impl Drop for SoftwareEncoder {
    fn drop(&mut self) {
        // Closing the queue ends the worker once it has finished what it is doing.
        self.jobs = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn worker(mut encoder: Encoder, jobs: &Receiver<Job>, done: &Sender<Done>) {
    while let Ok(job) = jobs.recv() {
        let result = match job {
            Job::Encode(picture, pts) => {
                let packets = {
                    let yuv = YuvFrame {
                        y: &picture.y,
                        u: &picture.u,
                        v: &picture.v,
                        y_stride: picture.width as usize,
                        uv_stride: picture.width as usize / 2,
                    };
                    encoder.try_encode(&yuv, pts)
                };
                match packets {
                    Ok(packets) => Done::Frame { packets, picture },
                    Err(_) => Done::Failed,
                }
            }
            Job::Flush => Done::Flushed(encoder.flush()),
        };
        if done.send(result).is_err() {
            break;
        }
    }
}
