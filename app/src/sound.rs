// SPDX-License-Identifier: FSL-1.1-ALv2
//! Sounds from the computer's speakers (D-059), while no puck is connected. The engine says what
//! to play ([`Engine::sounds`]); [`hn_engine::tones`] renders it like the puck's firmware does.
//! The audio device is only opened while a sound plays, so it isn't kept awake.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};
use hn_engine::{here::Sound, log, Engine};
use std::time::Duration;

pub fn start(engine: &Engine) {
    let mut rx = engine.sounds();
    let (tx, queue) = std::sync::mpsc::channel::<Sound>();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(s) => {
                    let _ = tx.send(s);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
    // cpal streams aren't Send everywhere: one thread plays them, one after another.
    std::thread::spawn(move || {
        while let Ok(s) = queue.recv() {
            if let Err(e) = play(&s) {
                log!("sound {}: {e}", s.sound);
            }
        }
    });
}

fn play(s: &Sound) -> Result<(), String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no audio output")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let Some(samples) = hn_engine::tones::render(&s.sound, s.gain, s.repeat, config.sample_rate)
    else {
        return Ok(());
    };
    let secs = samples.len() as f64 / config.sample_rate as f64;
    let stream = match format {
        SampleFormat::F32 => run::<f32>(&device, &config, samples)?,
        SampleFormat::I16 => run::<i16>(&device, &config, samples)?,
        SampleFormat::U16 => run::<u16>(&device, &config, samples)?,
        SampleFormat::I32 => run::<i32>(&device, &config, samples)?,
        SampleFormat::F64 => run::<f64>(&device, &config, samples)?,
        f => return Err(format!("sample format {f} not supported")),
    };
    // Keep the stream until the sound and the device's buffer have played out.
    std::thread::sleep(Duration::from_secs_f64(secs) + Duration::from_millis(250));
    drop(stream);
    Ok(())
}

fn run<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    samples: Vec<f32>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let mut i = 0usize;
    let stream = device
        .build_output_stream::<T, _, _>(
            *config,
            move |out: &mut [T], _| {
                for frame in out.chunks_mut(channels) {
                    let v = samples.get(i).copied().unwrap_or(0.0);
                    i += 1;
                    for x in frame.iter_mut() {
                        *x = T::from_sample(v);
                    }
                }
            },
            |e| log!("sound: stream error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}
