use leptos::{
    component,
    create_effect,
    create_node_ref,
    html::Textarea,
    on_cleanup,
    store_value,
    view,
    IntoView,
    ReadSignal,
    RwSignal,
    SignalSet,
    SignalWithUntracked,
};
use serde::Serialize;
use wasm_bindgen::{
    prelude::Closure,
    JsCast,
    JsValue,
};

#[component]
pub fn CodeMirror(contents: RwSignal<String>, options: ReadSignal<EditorOptions>) -> impl IntoView {
    let text_area_node_ref = create_node_ref::<Textarea>();
    let on_change_closure = store_value(None);

    create_effect(move |_| {
        tracing::debug!("textarea loaded");
        let Some(text_area) = text_area_node_ref.get()
        else {
            return;
        };

        tracing::debug!("attaching editor to textarea");
        let options = options.with_untracked(|options| JsValue::from(options));
        let editor = code_mirror_sys::from_text_area(&text_area, &options);
        editor.set_value(&contents.with_untracked(|contents| JsValue::from(contents)));

        let closure = Closure::wrap(Box::new(
            move |editor: code_mirror_sys::Editor, _value: JsValue| {
                //let change = ChangeObject::try_from(value).unwrap();
                contents.set(String::try_from(editor.get_value()).unwrap());
            },
        )
            as Box<dyn FnMut(code_mirror_sys::Editor, JsValue)>);
        editor.on("change", closure.as_ref().unchecked_ref());
        on_change_closure.set_value(Some(closure));
    });

    on_cleanup(move || {
        on_change_closure.update_value(|opt| {
            if let Some(closure) = opt.take() {
                closure.forget();
            }
        });
    });

    view! {
        <div>
            <style>r#"
                .CodeMirror {
                    width: 100%;
                    height: 100%;
                }
            "#</style>
            <textarea node_ref=text_area_node_ref></textarea>
        </div>
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditorOptions {
    pub line_numbers: bool,
}

impl EditorOptions {
    pub fn line_numbers(mut self, v: bool) -> Self {
        self.line_numbers = v;
        self
    }
}

impl From<&EditorOptions> for JsValue {
    fn from(value: &EditorOptions) -> Self {
        serde_wasm_bindgen::to_value(value).unwrap()
    }
}

mod code_mirror_sys {
    use wasm_bindgen::{
        prelude::wasm_bindgen,
        JsValue,
    };
    use web_sys::HtmlTextAreaElement;

    #[wasm_bindgen]
    extern "C" {

        #[derive(Debug)]
        pub type Doc;

        #[derive(Debug)]
        pub type LineHandle;

        #[wasm_bindgen(method, js_name = getEditor)]
        pub fn get_editor(this: &Doc) -> Editor;

        #[wasm_bindgen(method, js_name = getValue)]
        pub fn get_value(this: &Doc) -> JsValue;

        #[wasm_bindgen(method, js_name = setValue)]
        pub fn set_value(this: &Doc, text: &JsValue);

        #[derive(Debug)]
        #[wasm_bindgen(extends = Doc)]
        pub type Editor;

        #[wasm_bindgen(method, js_name = getDoc)]
        pub fn get_doc(this: &Editor) -> Doc;

        #[wasm_bindgen(method)]
        pub fn save(this: &Editor);

        #[wasm_bindgen(js_name = fromTextArea, js_namespace = CodeMirror)]
        pub fn from_text_area(text_area: &HtmlTextAreaElement, options: &JsValue) -> Editor;

        #[wasm_bindgen(method, js_name = on)]
        pub fn on(this: &Editor, event_name: &str, callback: &JsValue);

        #[wasm_bindgen(method, js_name = setSize)]
        pub fn set_size(this: &Editor, width: &JsValue, height: &JsValue);

    }
}
