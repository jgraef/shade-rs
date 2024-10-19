use kardashev_style::style;
use leptos::{
    component,
    create_effect,
    create_node_ref,
    html::{
        Canvas,
        Div,
    },
    on_cleanup,
    provide_context,
    store_value,
    use_context,
    view,
    IntoView,
    Signal,
    SignalGet,
    SignalGetUntracked,
};
use leptos_use::{
    signal_debounced,
    use_document_visibility,
    use_element_size_with_options,
    use_element_visibility,
    UseElementSizeOptions,
};
use web_sys::{
    ResizeObserverBoxOptions,
    VisibilityState,
};

use crate::graphics::{
    self,
    FrameInfo,
    Graphics,
    SurfaceSize,
    WindowHandle,
    WindowId,
};

#[style(path = "src/app/window.scss")]
struct Style;

pub fn use_graphics() -> Graphics {
    use_context::<Graphics>().unwrap_or_else(|| {
        let graphics = Graphics::new(graphics::Config {
            power_preference: Default::default(),
            backend_type: graphics::SelectBackendType::AutoDetect,
        });
        provide_context(graphics.clone());
        graphics
    })
}

/// A window (i.e. a HTML canvas) to which a scene is rendered.
/// This creates a container (div) that can be sized using CSS. The canvas will
/// atomatically be resized to fill this container.
///
/// # TODO
///
/// - Add event handler property
#[component]
pub fn Window<OnLoad, OnFrame>(on_load: OnLoad, on_frame: OnFrame) -> impl IntoView
where
    OnLoad: FnOnce(WindowHandle) + 'static,
    OnFrame: FnMut(FrameInfo) + 'static,
{
    let container_node_ref = create_node_ref::<Div>();
    let canvas_node_ref = create_node_ref::<Canvas>();
    let stored_window_handle = store_value(None);

    let container_size = use_element_size_with_options(
        container_node_ref,
        UseElementSizeOptions::default().box_(ResizeObserverBoxOptions::ContentBox),
    );
    let container_size = signal_debounced(
        Signal::derive(move || {
            SurfaceSize {
                width: (container_size.width.get() as u32).max(1),
                height: (container_size.height.get() as u32).max(1),
            }
        }),
        500.,
    );

    let window_id = WindowId::new();

    canvas_node_ref.on_load(move |_canvas| {
        tracing::debug!("window loaded");
        let window_handle = use_graphics().register_window(
            window_id,
            container_size.get_untracked(),
            Box::new(on_frame),
        );
        stored_window_handle.set_value(Some(window_handle.clone()));
        on_load(window_handle);
    });

    create_effect(move |_| {
        let surface_size = container_size.get();
        tracing::debug!(?surface_size, "container resized");
        stored_window_handle.with_value(|window_handle_opt| {
            if let Some(window_handle) = window_handle_opt {
                window_handle.resize(surface_size);
            }
        });
    });

    let element_visibility = use_element_visibility(container_node_ref);
    let document_visibility = use_document_visibility();
    let is_visible = Signal::derive(move || {
        element_visibility.get() && document_visibility.get() == VisibilityState::Visible
    });
    create_effect(move |_| {
        let visible = is_visible.get();

        stored_window_handle.with_value(|window_handle_opt| {
            if let Some(window_handle) = window_handle_opt {
                window_handle.set_visibility(visible);
            }
        });
    });

    on_cleanup(move || {
        stored_window_handle.with_value(|window_handle_opt| {
            if let Some(window_handle) = window_handle_opt {
                window_handle.destroy_window();
            }
        });
    });

    view! {
        <div
            node_ref=container_node_ref
            class=Style::window
        >
            <canvas
                node_ref=canvas_node_ref
                width=move || container_size.get().width
                height=move || container_size.get().height
                data-raw-handle=window_id
                on:mousemove=move |event| {
                    stored_window_handle.with_value(|window_handle_opt| {
                        if let Some(window_handle) = window_handle_opt {
                            window_handle.set_mouse_position(Some(mouse_position_from_websys(&event)));
                        }
                    });
                }
                on:mouseleave=move |_event| {
                    stored_window_handle.with_value(|window_handle_opt| {
                        if let Some(window_handle) = window_handle_opt {
                            window_handle.set_mouse_position(None);
                        }
                    });
                }
            ></canvas>
        </div>
    }
}

fn mouse_position_from_websys(event: &web_sys::MouseEvent) -> [f32; 2] {
    [event.offset_x() as f32, event.offset_y() as f32]
}
