use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use log::{error, info};
use rubato::{FastFixedIn, PolynomialDegree, Resampler};

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

pub struct CapturedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

pub struct AudioCapture {
    stop_tx: mpsc::Sender<()>,
    thread: JoinHandle<CapturedAudio>,
}

impl AudioCapture {
    pub fn start() -> Result<AudioCapture, String> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        let thread = std::thread::spawn(move || -> CapturedAudio {
            let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
            let empty = || CapturedAudio { samples: Vec::new(), sample_rate: TARGET_SAMPLE_RATE };

            let host = cpal::default_host();
            let device = match host.default_input_device() {
                Some(d) => d,
                None => {
                    let _ = ready_tx.send(Err("no default input device".into()));
                    return empty();
                }
            };

            let supported = match device.default_input_config() {
                Ok(c) => c,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("default_input_config: {e}")));
                    return empty();
                }
            };
            let source_rate = supported.sample_rate().0;
            let source_channels = supported.channels() as usize;
            let stream_config: cpal::StreamConfig = supported.config();
            info!(
                "audio: source rate {source_rate} Hz, {source_channels} channel(s) -> mono @ {TARGET_SAMPLE_RATE}"
            );

            let samples_inner = samples.clone();
            let stream = match device.build_input_stream(
                &stream_config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let Ok(mut buf) = samples_inner.lock() else { return };
                    if source_channels <= 1 {
                        buf.extend_from_slice(data);
                    } else {
                        for frame in data.chunks_exact(source_channels) {
                            let avg: f32 = frame.iter().copied().sum::<f32>() / source_channels as f32;
                            buf.push(avg);
                        }
                    }
                },
                |err| error!("audio stream error: {err}"),
                None,
            ) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(format!("build_input_stream: {e}")));
                    return empty();
                }
            };

            if let Err(e) = stream.play() {
                let _ = ready_tx.send(Err(format!("stream.play: {e}")));
                return empty();
            }

            let _ = ready_tx.send(Ok(()));
            let _ = stop_rx.recv();
            drop(stream);
            CapturedAudio {
                samples: std::mem::take(&mut *samples.lock().unwrap()),
                sample_rate: source_rate,
            }
        });

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(AudioCapture { stop_tx, thread }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("audio thread crashed before ready".into()),
        }
    }

    pub fn stop(self) -> CapturedAudio {
        let _ = self.stop_tx.send(());
        self.thread.join().unwrap_or_else(|_| CapturedAudio {
            samples: Vec::new(),
            sample_rate: TARGET_SAMPLE_RATE,
        })
    }
}

pub fn encode_wav(samples: &[f32], source_rate: u32) -> Result<Vec<u8>, String> {
    let mono16k = resample_to_target(samples, source_rate)?;
    let mut buf = std::io::Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    {
        let mut writer = hound::WavWriter::new(&mut buf, spec).map_err(|e| e.to_string())?;
        for s in mono16k {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer.write_sample(v).map_err(|e| e.to_string())?;
        }
        writer.finalize().map_err(|e| e.to_string())?;
    }
    Ok(buf.into_inner())
}

fn resample_to_target(input: &[f32], source_rate: u32) -> Result<Vec<f32>, String> {
    if source_rate == TARGET_SAMPLE_RATE || input.is_empty() {
        return Ok(input.to_vec());
    }
    let ratio = TARGET_SAMPLE_RATE as f64 / source_rate as f64;
    let mut resampler =
        FastFixedIn::<f32>::new(ratio, 2.0, PolynomialDegree::Cubic, input.len(), 1)
            .map_err(|e| format!("resampler init: {e}"))?;
    let waves_in = vec![input.to_vec()];
    let waves_out = resampler
        .process(&waves_in, None)
        .map_err(|e| format!("resample: {e}"))?;
    waves_out
        .into_iter()
        .next()
        .ok_or_else(|| "empty resampler output".to_string())
}
