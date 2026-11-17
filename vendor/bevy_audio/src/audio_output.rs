use crate::{
    AudioPlayer, Decodable, DefaultSpatialScale, GlobalVolume, PlaybackMode, PlaybackSettings,
    SpatialAudioSink, SpatialListener,
};
use bevy_asset::{Asset, Assets};
use bevy_ecs::{prelude::*, system::SystemParam};
use bevy_math::Vec3;
use bevy_transform::prelude::GlobalTransform;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player, Source, SpatialPlayer};
#[cfg(not(target_arch = "wasm32"))]
use tracing::info;
use tracing::warn;
#[cfg(not(target_arch = "wasm32"))]
use alloc::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use core::sync::atomic::{AtomicBool, Ordering};

use crate::{AudioSink, AudioSinkPlayback};

/// Used internally to play audio on the current "audio device"
///
/// Space Guard: the stream follows the system default output device. When the device is lost
/// (headphones disconnected) or the default device changes (headphones reconnected), the stream is
/// reopened by [`follow_output_device`].
#[derive(Resource)]
pub(crate) struct AudioOutput {
    stream: Option<MixerDeviceSink>,
    /// Device the stream is open on.
    #[cfg(not(target_arch = "wasm32"))]
    device: Option<rodio::cpal::DeviceId>,
    /// Set by the stream error callback when the device is gone.
    #[cfg(not(target_arch = "wasm32"))]
    lost: Arc<AtomicBool>,
    /// When the device was last checked.
    #[cfg(not(target_arch = "wasm32"))]
    checked: std::time::Instant,
}

impl Default for AudioOutput {
    #[cfg(target_arch = "wasm32")]
    fn default() -> Self {
        let stream = DeviceSinkBuilder::open_default_sink()
            .inspect_err(|_err| {
                warn!("No audio device found.");
            })
            .map(|mut s| {
                s.log_on_drop(false);
                s
            })
            .ok();
        Self { stream }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn default() -> Self {
        let lost = Arc::new(AtomicBool::new(false));
        let (stream, device) = open_default(&lost);
        if stream.is_none() {
            warn!("No audio device found.");
        }
        Self {
            stream,
            device,
            lost,
            checked: std::time::Instant::now(),
        }
    }
}

/// Opens the default output device; the error callback raises `lost` when the device is gone.
#[cfg(not(target_arch = "wasm32"))]
fn open_default(lost: &Arc<AtomicBool>) -> (Option<MixerDeviceSink>, Option<rodio::cpal::DeviceId>) {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

    lost.store(false, Ordering::Relaxed);
    let Some(device) = rodio::cpal::default_host().default_output_device() else {
        return (None, None);
    };
    let id = device.id().ok();
    let flag = lost.clone();
    let stream = DeviceSinkBuilder::from_device(device)
        .and_then(|builder| {
            builder
                .with_error_callback(move |err| {
                    if matches!(
                        err,
                        rodio::cpal::StreamError::DeviceNotAvailable
                            | rodio::cpal::StreamError::StreamInvalidated
                    ) {
                        flag.store(true, Ordering::Relaxed);
                    }
                    warn!("audio stream error: {err}");
                })
                .open_stream()
        })
        .or_else(|_| DeviceSinkBuilder::open_default_sink())
        .map(|mut s| {
            s.log_on_drop(false);
            s
        })
        .ok();
    (stream, id)
}

/// How often the default output device is checked.
#[cfg(not(target_arch = "wasm32"))]
const DEVICE_CHECK: core::time::Duration = core::time::Duration::from_secs(1);

/// Space Guard: reopens the output when the device is lost or the system default device changes.
/// Looping sounds restart on the new device; other playing sounds stop and are cleaned up as finished.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn follow_output_device(
    mut audio_output: ResMut<AudioOutput>,
    mut commands: Commands,
    sinks: Query<(
        Entity,
        &PlaybackSettings,
        Option<&AudioSink>,
        Option<&SpatialAudioSink>,
    )>,
) {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};

    if audio_output.checked.elapsed() < DEVICE_CHECK {
        return;
    }
    audio_output.checked = std::time::Instant::now();
    let lost = audio_output.lost.load(Ordering::Relaxed);
    let current = rodio::cpal::default_host()
        .default_output_device()
        .and_then(|device| device.id().ok());
    let changed = current != audio_output.device;
    if !lost && !changed && audio_output.stream.is_some() {
        return;
    }
    if current.is_none() && audio_output.stream.is_none() {
        return;
    }
    // The old stream goes first: some backends refuse a second stream on the same device.
    audio_output.stream = None;
    let flag = audio_output.lost.clone();
    let (stream, device) = open_default(&flag);
    audio_output.device = device;
    let Some(_) = stream else {
        warn!("Audio output lost, no device to reopen on.");
        return;
    };
    audio_output.stream = stream;
    info!("Audio output reopened on the current default device.");
    for (entity, settings, sink, spatial) in &sinks {
        if sink.is_none() && spatial.is_none() {
            continue;
        }
        if matches!(settings.mode, PlaybackMode::Loop) {
            commands.entity(entity).remove::<(AudioSink, SpatialAudioSink)>();
        } else {
            if let Some(sink) = sink {
                sink.stop();
            }
            if let Some(spatial) = spatial {
                spatial.stop();
            }
        }
    }
}

/// Marker for internal use, to despawn entities when playback finishes.
#[derive(Component, Default)]
pub struct PlaybackDespawnMarker;

/// Marker for internal use, to remove audio components when playback finishes.
#[derive(Component, Default)]
pub struct PlaybackRemoveMarker;

#[derive(SystemParam)]
pub(crate) struct EarPositions<'w, 's> {
    pub(crate) query: Query<'w, 's, (Entity, &'static GlobalTransform, &'static SpatialListener)>,
}

impl<'w, 's> EarPositions<'w, 's> {
    /// Gets a set of transformed ear positions.
    ///
    /// If there are no listeners, use the default values. If a user has added multiple
    /// listeners for whatever reason, we will return the first value.
    pub(crate) fn get(&self) -> (Vec3, Vec3) {
        let (left_ear, right_ear) = self
            .query
            .iter()
            .next()
            .map(|(_, transform, settings)| {
                (
                    transform.transform_point(settings.left_ear_offset),
                    transform.transform_point(settings.right_ear_offset),
                )
            })
            .unwrap_or_else(|| {
                let settings = SpatialListener::default();
                (settings.left_ear_offset, settings.right_ear_offset)
            });

        (left_ear, right_ear)
    }

    pub(crate) fn multiple_listeners(&self) -> bool {
        self.query.iter().len() > 1
    }
}

/// Plays "queued" audio through the [`AudioOutput`] resource.
///
/// "Queued" audio is any audio entity (with an [`AudioPlayer`] component) that does not have an
/// [`AudioSink`]/[`SpatialAudioSink`] component.
///
/// This system detects such entities, checks if their source asset
/// data is available, and creates/inserts the sink.
pub(crate) fn play_queued_audio_system<Source: Asset + Decodable>(
    audio_output: Res<AudioOutput>,
    audio_sources: Res<Assets<Source>>,
    global_volume: Res<GlobalVolume>,
    query_nonplaying: Query<
        (
            Entity,
            &AudioPlayer<Source>,
            &PlaybackSettings,
            Option<&GlobalTransform>,
        ),
        (Without<AudioSink>, Without<SpatialAudioSink>),
    >,
    ear_positions: EarPositions,
    default_spatial_scale: Res<DefaultSpatialScale>,
    mut commands: Commands,
) where
    f32: rodio::cpal::FromSample<rodio::Sample>,
{
    let Some(stream) = audio_output.stream.as_ref() else {
        // audio output unavailable; cannot play sound
        return;
    };
    let mixer = stream.mixer();

    for (entity, source_handle, settings, maybe_emitter_transform) in &query_nonplaying {
        let Some(audio_source) = audio_sources.get(&source_handle.0) else {
            continue;
        };
        // audio data is available (has loaded), begin playback and insert sink component
        if settings.spatial {
            let (left_ear, right_ear) = ear_positions.get();

            // We can only use one `SpatialListener`. If there are more than that, then
            // the user may have made a mistake.
            if ear_positions.multiple_listeners() {
                warn!(
                    "Multiple SpatialListeners found. Using {}.",
                    ear_positions.query.iter().next().unwrap().0
                );
            }

            let scale = settings.spatial_scale.unwrap_or(default_spatial_scale.0).0;

            let emitter_translation = if let Some(emitter_transform) = maybe_emitter_transform {
                (emitter_transform.translation() * scale).into()
            } else {
                warn!("Spatial AudioPlayer with no GlobalTransform component. Using zero.");
                Vec3::ZERO.into()
            };

            let sink = SpatialPlayer::connect_new(
                mixer,
                emitter_translation,
                (left_ear * scale).into(),
                (right_ear * scale).into(),
            );

            let decoder = audio_source.decoder();

            match settings.mode {
                PlaybackMode::Loop => match (settings.start_position, settings.duration) {
                    // custom start position and duration
                    (Some(start_position), Some(duration)) => sink.append(
                        decoder
                            .skip_duration(start_position)
                            .take_duration(duration)
                            .repeat_infinite(),
                    ),

                    // custom start position
                    (Some(start_position), None) => {
                        sink.append(decoder.skip_duration(start_position).repeat_infinite());
                    }

                    // custom duration
                    (None, Some(duration)) => {
                        sink.append(decoder.take_duration(duration).repeat_infinite());
                    }

                    // full clip
                    (None, None) => sink.append(decoder.repeat_infinite()),
                },
                PlaybackMode::Once | PlaybackMode::Despawn | PlaybackMode::Remove => {
                    match (settings.start_position, settings.duration) {
                        (Some(start_position), Some(duration)) => sink.append(
                            decoder
                                .skip_duration(start_position)
                                .take_duration(duration),
                        ),

                        (Some(start_position), None) => {
                            sink.append(decoder.skip_duration(start_position));
                        }

                        (None, Some(duration)) => sink.append(decoder.take_duration(duration)),

                        (None, None) => sink.append(decoder),
                    }
                }
            }

            let mut sink = SpatialAudioSink::new(sink);

            if settings.muted {
                sink.mute();
            }

            sink.set_speed(settings.speed);
            sink.set_volume(settings.volume * global_volume.volume);

            if settings.paused {
                sink.pause();
            }

            match settings.mode {
                PlaybackMode::Loop | PlaybackMode::Once => commands.entity(entity).insert(sink),
                PlaybackMode::Despawn => commands
                    .entity(entity)
                    // PERF: insert as bundle to reduce archetype moves
                    .insert((sink, PlaybackDespawnMarker)),
                PlaybackMode::Remove => commands
                    .entity(entity)
                    // PERF: insert as bundle to reduce archetype moves
                    .insert((sink, PlaybackRemoveMarker)),
            };
        } else {
            let sink = Player::connect_new(mixer);
            let decoder = audio_source.decoder();

            match settings.mode {
                PlaybackMode::Loop => match (settings.start_position, settings.duration) {
                    // custom start position and duration
                    (Some(start_position), Some(duration)) => sink.append(
                        decoder
                            .skip_duration(start_position)
                            .take_duration(duration)
                            .repeat_infinite(),
                    ),

                    // custom start position
                    (Some(start_position), None) => {
                        sink.append(decoder.skip_duration(start_position).repeat_infinite());
                    }

                    // custom duration
                    (None, Some(duration)) => {
                        sink.append(decoder.take_duration(duration).repeat_infinite());
                    }

                    // full clip
                    (None, None) => sink.append(decoder.repeat_infinite()),
                },
                PlaybackMode::Once | PlaybackMode::Despawn | PlaybackMode::Remove => {
                    match (settings.start_position, settings.duration) {
                        (Some(start_position), Some(duration)) => sink.append(
                            decoder
                                .skip_duration(start_position)
                                .take_duration(duration),
                        ),

                        (Some(start_position), None) => {
                            sink.append(decoder.skip_duration(start_position));
                        }

                        (None, Some(duration)) => sink.append(decoder.take_duration(duration)),

                        (None, None) => sink.append(decoder),
                    }
                }
            }

            let mut sink = AudioSink::new(sink);

            if settings.muted {
                sink.mute();
            }

            sink.set_speed(settings.speed);
            sink.set_volume(settings.volume * global_volume.volume);

            if settings.paused {
                sink.pause();
            }

            match settings.mode {
                PlaybackMode::Loop | PlaybackMode::Once => commands.entity(entity).insert(sink),
                PlaybackMode::Despawn => commands
                    .entity(entity)
                    // PERF: insert as bundle to reduce archetype moves
                    .insert((sink, PlaybackDespawnMarker)),
                PlaybackMode::Remove => commands
                    .entity(entity)
                    // PERF: insert as bundle to reduce archetype moves
                    .insert((sink, PlaybackRemoveMarker)),
            };
        }
    }
}

pub(crate) fn cleanup_finished_audio<T: Decodable + Asset>(
    mut commands: Commands,
    query_nonspatial_remove: Query<
        (Entity, &AudioSink),
        (With<PlaybackRemoveMarker>, With<AudioPlayer<T>>),
    >,
    query_spatial_remove: Query<
        (Entity, &SpatialAudioSink),
        (With<PlaybackRemoveMarker>, With<AudioPlayer<T>>),
    >,
) {
    commands.despawn_all_where::<
        AnyOf<(&AudioSink, &SpatialAudioSink)>,
        (With<PlaybackDespawnMarker>, With<AudioPlayer<T>>)
    >(
        |(sink, spatial)|
            sink.is_some_and(AudioSinkPlayback::empty) || spatial.is_some_and(AudioSinkPlayback::empty)
    );

    for (entity, sink) in &query_nonspatial_remove {
        if sink.sink.empty() {
            commands.entity(entity).remove::<(
                AudioPlayer<T>,
                AudioSink,
                PlaybackSettings,
                PlaybackRemoveMarker,
            )>();
        }
    }
    for (entity, sink) in &query_spatial_remove {
        if sink.sink.empty() {
            commands.entity(entity).remove::<(
                AudioPlayer<T>,
                SpatialAudioSink,
                PlaybackSettings,
                PlaybackRemoveMarker,
            )>();
        }
    }
}

/// Run Condition to only play audio if the audio output is available
pub(crate) fn audio_output_available(audio_output: Res<AudioOutput>) -> bool {
    audio_output.stream.is_some()
}

/// Updates spatial audio sinks when emitter positions change.
pub(crate) fn update_emitter_positions(
    mut emitters: Query<
        (&GlobalTransform, &SpatialAudioSink, &PlaybackSettings),
        Or<(Changed<GlobalTransform>, Changed<PlaybackSettings>)>,
    >,
    default_spatial_scale: Res<DefaultSpatialScale>,
) {
    for (transform, sink, settings) in emitters.iter_mut() {
        let scale = settings.spatial_scale.unwrap_or(default_spatial_scale.0).0;

        let translation = transform.translation() * scale;
        sink.set_emitter_position(translation);
    }
}

/// Updates spatial audio sink ear positions when spatial listeners change.
pub(crate) fn update_listener_positions(
    mut emitters: Query<(&SpatialAudioSink, &PlaybackSettings)>,
    changed_listener: Query<
        (),
        (
            Or<(
                Changed<SpatialListener>,
                Changed<GlobalTransform>,
                Changed<PlaybackSettings>,
            )>,
            With<SpatialListener>,
        ),
    >,
    ear_positions: EarPositions,
    default_spatial_scale: Res<DefaultSpatialScale>,
) {
    if !default_spatial_scale.is_changed() && changed_listener.is_empty() {
        return;
    }

    let (left_ear, right_ear) = ear_positions.get();

    for (sink, settings) in emitters.iter_mut() {
        let scale = settings.spatial_scale.unwrap_or(default_spatial_scale.0).0;

        sink.set_ears_position(left_ear * scale, right_ear * scale);
    }
}
