//! Реплики и музыка в браузере играет сам браузер.
//!
//! Браузерная сборка однопоточная: общий микшер (`rodio` поверх WebAudio-вывода `cpal`) заполняет
//! буферы по ~46 мс в главном потоке и раскодирует в них все звучащие mp3. Кадр длиннее буфера —
//! щель в звуке, в сплошной речи её слышно сразу. Поэтому в браузере голос и музыку играет
//! аудиоэлемент страницы: он раскодирует и ведёт поток в звуковом потоке браузера, кадры игры его
//! не задерживают. Его выход идёт через узел громкости (`MediaElementAudioSourceNode → GainNode`):
//! свойство `volume` аудиоэлемента на iOS не работает. Эффекты остаются в общем микшере.
//!
//! Код игры спавнит проигрыватель через [`player`]: в браузере это [`WebSound`], на остальных
//! платформах — обычные `AudioPlayer` и `PlaybackSettings`. Громкость звучащего звука — поле
//! [`WebSound::volume`]; неповторяющийся звук, доиграв, удаляет свою сущность сам, как
//! `PlaybackSettings::DESPAWN`.

use bevy::audio::{AudioSource, Volume};
use bevy::prelude::*;

/// Проигрыватель реплик и музыки браузера.
pub struct WebAudioPlugin;

impl Plugin for WebAudioPlugin {
    fn build(&self, _app: &mut App) {
        // После `Update`: голос и музыка спавнят и меняют звуки там.
        #[cfg(target_arch = "wasm32")]
        _app.add_systems(
            PostUpdate,
            (browser::stop_removed, browser::start, browser::follow_volume, browser::finish).chain(),
        );
    }
}

/// Звук, который играет браузер: файл ассета, громкость и повтор.
#[derive(Component, Debug, Clone)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub struct WebSound {
    pub source: Handle<AudioSource>,
    /// Линейная громкость; изменение сразу слышно у звучащего звука.
    pub volume: f32,
    pub looped: bool,
}

/// Браузер начал звук [`WebSound`]: дальше его сущность удалит проигрыватель, когда звук кончится.
#[derive(Component)]
pub struct WebPlaying;

/// Проигрыватель реплики или музыки для платформы: в браузере — [`WebSound`].
#[cfg(target_arch = "wasm32")]
pub fn player(source: Handle<AudioSource>, volume: f32, looped: bool) -> impl Bundle {
    WebSound { source, volume, looped }
}

/// Проигрыватель реплики или музыки для платформы: вне браузера — общий микшер.
#[cfg(not(target_arch = "wasm32"))]
pub fn player(source: Handle<AudioSource>, volume: f32, looped: bool) -> impl Bundle {
    (
        AudioPlayer::new(source),
        PlaybackSettings {
            volume: Volume::Linear(volume),
            ..if looped { PlaybackSettings::LOOP } else { PlaybackSettings::DESPAWN }
        },
    )
}

/// Ставит громкость звучащему проигрывателю любого вида: в браузере — [`WebSound`], иначе — `AudioSink`.
pub fn set_volume(sink: Option<Mut<AudioSink>>, web: Option<Mut<WebSound>>, volume: f32) {
    use bevy::audio::AudioSinkPlayback;
    if let Some(mut sink) = sink {
        sink.set_volume(Volume::Linear(volume));
    }
    if let Some(mut web) = web
        && web.volume != volume
    {
        web.volume = volume;
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    use bevy::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{AudioContext, GainNode, HtmlAudioElement};

    use super::{WebPlaying, WebSound};

    /// Звучащий аудиоэлемент и его узел громкости.
    struct Playing {
        element: HtmlAudioElement,
        gain: GainNode,
        /// Обработчик `ended`: живёт, пока живёт звук.
        _ended: Closure<dyn FnMut()>,
    }

    #[derive(Default)]
    struct State {
        context: Option<AudioContext>,
        playing: HashMap<Entity, Playing>,
        /// Сущности доигравших звуков: обработчик `ended` кладёт, [`finish`] удаляет.
        ended: Rc<RefCell<Vec<Entity>>>,
        /// Обработчик `visibilitychange`, ставится один раз.
        visibility: Option<Closure<dyn FnMut()>>,
        /// Обработчик отказа `play()`: без него браузер пишет в консоль необработанную ошибку.
        ignore: Option<Closure<dyn FnMut(wasm_bindgen::JsValue)>>,
    }

    thread_local! {
        static STATE: RefCell<State> = RefCell::new(State::default());
    }

    /// Свой `AudioContext`. Страница (`web/index.html`) запоминает каждый созданный контекст и будит
    /// его на касании и нажатии «начать».
    fn context(state: &mut State) -> Option<AudioContext> {
        if state.context.is_none() {
            state.context = AudioContext::new().inspect_err(|err| warn!("Звук браузера: нет AudioContext: {err:?}")).ok();
        }
        state.context.clone()
    }

    /// Аудиоэлементы на паузе, пока вкладка скрыта: кадры игры там не идут, реплика ушла бы вперёд.
    fn watch_visibility(state: &mut State) {
        if state.visibility.is_some() {
            return;
        }
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let hook = Closure::<dyn FnMut()>::new(|| {
            let hidden = web_sys::window().and_then(|window| window.document()).is_some_and(|doc| doc.hidden());
            STATE.with_borrow(|state| {
                for playing in state.playing.values() {
                    if hidden {
                        let _ = playing.element.pause();
                    } else {
                        play(state, &playing.element);
                    }
                }
            });
        });
        let _ = document.add_event_listener_with_callback("visibilitychange", hook.as_ref().unchecked_ref());
        state.visibility = Some(hook);
    }

    fn play(state: &State, element: &HtmlAudioElement) {
        if let (Ok(promise), Some(ignore)) = (element.play(), &state.ignore) {
            let _ = promise.catch(ignore);
        }
    }

    /// Начинает звуки, которые браузер ещё не играет.
    pub fn start(
        mut commands: Commands,
        asset_server: Res<AssetServer>,
        sounds: Query<(Entity, &WebSound), Without<WebPlaying>>,
    ) {
        if sounds.is_empty() {
            return;
        }
        STATE.with_borrow_mut(|state| {
            let Some(context) = context(state) else {
                return;
            };
            watch_visibility(state);
            state.ignore.get_or_insert_with(|| Closure::new(|_| {}));
            for (entity, sound) in &sounds {
                let Some(path) = asset_server.get_path(sound.source.id()) else {
                    warn!("Звук браузера: у звука нет пути ассета");
                    commands.entity(entity).despawn();
                    continue;
                };
                let url = format!("assets/{}", path.path().to_string_lossy());
                let Some(playing) = connect(&context, &url, sound, entity, &state.ended) else {
                    warn!("Звук браузера: «{url}» не запустился");
                    commands.entity(entity).despawn();
                    continue;
                };
                play(state, &playing.element);
                state.playing.insert(entity, playing);
                commands.entity(entity).insert(WebPlaying);
            }
        });
    }

    /// Аудиоэлемент с файлом, подключённый к контексту через узел громкости.
    fn connect(
        context: &AudioContext,
        url: &str,
        sound: &WebSound,
        entity: Entity,
        ended: &Rc<RefCell<Vec<Entity>>>,
    ) -> Option<Playing> {
        let element = HtmlAudioElement::new_with_src(url).ok()?;
        element.set_loop(sound.looped);
        element.set_preload("auto");
        let source = context.create_media_element_source(&element).ok()?;
        let gain = context.create_gain().ok()?;
        gain.gain().set_value(sound.volume);
        source.connect_with_audio_node(&gain).ok()?;
        gain.connect_with_audio_node(&context.destination()).ok()?;
        let ended = ended.clone();
        let on_ended = Closure::<dyn FnMut()>::new(move || ended.borrow_mut().push(entity));
        element.set_onended(Some(on_ended.as_ref().unchecked_ref()));
        Some(Playing { element, gain, _ended: on_ended })
    }

    /// Громкость звучащих звуков.
    pub fn follow_volume(sounds: Query<(Entity, &WebSound), (Changed<WebSound>, With<WebPlaying>)>) {
        if sounds.is_empty() {
            return;
        }
        STATE.with_borrow(|state| {
            for (entity, sound) in &sounds {
                if let Some(playing) = state.playing.get(&entity) {
                    playing.gain.gain().set_value(sound.volume);
                }
            }
        });
    }

    /// Доигравшие звуки удаляют свою сущность.
    pub fn finish(mut commands: Commands) {
        STATE.with_borrow_mut(|state| {
            let ended: Vec<Entity> = state.ended.borrow_mut().drain(..).collect();
            for entity in ended {
                if let Some(playing) = state.playing.remove(&entity) {
                    release(playing);
                }
                commands.entity(entity).try_despawn();
            }
        });
    }

    /// Сущность звука удалена (реплику оборвала новая, трек отзатухал) — аудиоэлемент замолкает.
    pub fn stop_removed(mut removed: RemovedComponents<WebSound>) {
        let removed: Vec<Entity> = removed.read().collect();
        if removed.is_empty() {
            return;
        }
        STATE.with_borrow_mut(|state| {
            for entity in removed {
                if let Some(playing) = state.playing.remove(&entity) {
                    release(playing);
                }
            }
        });
    }

    /// Останавливает аудиоэлемент и отпускает файл: без `src` браузер освобождает поток.
    fn release(playing: Playing) {
        let _ = playing.element.pause();
        playing.element.set_onended(None);
        let _ = playing.element.remove_attribute("src");
        playing.element.load();
        let _ = playing.gain.disconnect();
    }
}
