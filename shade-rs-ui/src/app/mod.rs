mod code_mirror;
mod window;
mod icon;

use kardashev_style::style;
use leptos::{
    component,
    create_rw_signal,
    create_signal,
    spawn_local,
    store_value,
    view,
    IntoView,
    SignalGet,
    SignalGetUntracked,
    SignalSet,
    SignalWith,
};

use crate::{
    app::{
        code_mirror::{
            CodeMirror,
            EditorOptions,
        },
        
            icon::BootstrapIcon,
            window::Window,
        
    },
    graphics::{
        FrameInfo,
        WindowHandle,
    },
};

#[style(path = "src/app/app.scss")]
struct Style;

#[component]
pub fn App() -> impl IntoView {
    let window_handle = store_value::<Option<WindowHandle>>(None);

    let code = create_rw_signal(INITIAL_CODE.to_owned());
    let (options, _set_options) = create_signal(EditorOptions::default().line_numbers(true));
    //let code_debounced = signal_debounced(code, 1000.0);
    let frame_info = create_rw_signal(FrameInfo::default());
    let paused = create_rw_signal(false);
    let compiler_output = create_rw_signal::<Option<String>>(None);

    let run = move || {
        let Some(window_handle) = window_handle.get_value()
        else {
            return;
        };
        let code = code.get_untracked();
        spawn_local(async move {
            if let Err(error) = window_handle.run(code).await {
                compiler_output.set(Some(error.to_string()));
            }
            else {
                paused.set(false);
                compiler_output.set(None);
            }
        });
    };

    view! {
        <div class=Style::app>
            <div class=Style::preview>
                <Window
                    on_load=move |handle| {
                        window_handle.set_value(Some(handle));
                        if PLAY_ON_LOAD {
                            run();
                        }
                    }
                    on_frame=move |info| {
                        frame_info.set(info);
                    }
                />
            </div>
            <div class=Style::toolbar>
                <button
                    on:click=move |_| run()
                >
                    <BootstrapIcon icon="play-fill" />
                </button>
                <button
                    on:click=move |_| {
                        if let Some(window_handle) = window_handle.get_value() {
                            let new_value = !paused.get();
                            paused.set(new_value);
                            spawn_local(async move {
                                window_handle.set_paused(new_value);
                            });
                        }
                    }
                    data-toggled=move || paused.get()
                >
                    <BootstrapIcon icon="pause-fill" />
                </button>
                <button
                    on:click=move |_| {
                        if let Some(window_handle) = window_handle.get_value() {
                            spawn_local(async move {
                                window_handle.reset();
                            });
                        }
                    }
                >
                    <BootstrapIcon icon="skip-start-fill" />
                </button>
                <input
                    class=Style::time
                    type="text"
                    value=move || {
                        frame_info.with(|frame_info| format!("{:.3} s", frame_info.time))
                    }
                />
                <span class=Style::fps>
                {move || {
                    frame_info.with(|frame_info| format!("{:.1} FPS", frame_info.fps))
                }}
                </span>
            </div>
            <div
                class=Style::compiler_output
                data-hidden=move || compiler_output.with(|output| output.is_none())
            >
                {move || compiler_output.get().unwrap_or_default()}
            </div>
            <div class=Style::editor>
                <CodeMirror
                    contents=code
                    options
                />
            </div>
        </div>
    }
}

const INITIAL_CODE: &'static str = include_str!("shader.wgsl");
const PLAY_ON_LOAD: bool = true;
