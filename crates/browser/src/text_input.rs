use cef::rc::Rc as _;
use cef::{
    CefString, Domnode, Frame, ImplDomnode, ImplFrame, ImplListValue, ImplProcessMessage,
    ImplV8Context, ImplV8Exception, ImplV8Handler, ImplV8Value, ProcessId, ProcessMessage,
    V8Context, V8Handler, V8Value, WrapV8Handler, process_message_create,
    v8_context_get_current_context, v8_value_create_function, wrap_v8_handler,
};
use gpui::Keystroke;

pub(crate) const TEXT_INPUT_STATE_MESSAGE_NAME: &str = "glass.text_input_state";
pub(crate) const TEXT_INPUT_DEBUG_MESSAGE_NAME: &str = "glass.text_input_debug";
const TEXT_INPUT_DEBUG_BRIDGE_NAME: &str = "__glassReportTextInputDebug";
pub(crate) const TEXT_INPUT_DEBUG_DUMP_FN: &str = "__glassDumpActiveTextInput";

const TEXT_INPUT_DEBUG_OBSERVER_SCRIPT: &str = r#"
(function () {
  if (window.__glassTextInputDebugInstalled) return;
  window.__glassTextInputDebugInstalled = true;

  const bridge = window.__glassReportTextInputDebug;
  if (typeof bridge !== 'function') return;

  const snapshot = (reason, event) => {
    const active = document.activeElement;
    const payload = {
      reason,
      tag: active && active.tagName ? active.tagName : 'NONE',
      type: active && typeof active.type === 'string' ? active.type : '',
      isContentEditable: !!(active && active.isContentEditable),
      value: active && typeof active.value === 'string' ? active.value.slice(0, 200) : null,
      selectionStart:
        active && typeof active.selectionStart === 'number' ? active.selectionStart : null,
      selectionEnd:
        active && typeof active.selectionEnd === 'number' ? active.selectionEnd : null,
      eventType: event ? event.type : null,
      inputType: event && typeof event.inputType === 'string' ? event.inputType : null,
      data: event && typeof event.data === 'string' ? event.data : null,
      isComposing: !!(event && event.isComposing),
    };

    bridge(JSON.stringify(payload));
  };

  window.__glassDumpActiveTextInput = (reason) => snapshot(reason, null);

  for (const type of [
    'focusin',
    'beforeinput',
    'input',
    'change',
    'compositionstart',
    'compositionupdate',
    'compositionend',
  ]) {
    document.addEventListener(type, (event) => snapshot(type, event), true);
  }
})();
"#;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct BrowserTextInputState {
    pub(crate) editable: bool,
}

impl BrowserTextInputState {
    pub(crate) fn is_active(self, has_marked_text: bool) -> bool {
        self.editable || has_marked_text
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowserKeyDispatch {
    App,
    Browser,
    TextInput,
}

pub(crate) fn extract_text_input_state_from_message(
    message: &mut ProcessMessage,
) -> Option<BrowserTextInputState> {
    if CefString::from(&message.name()).to_string() != TEXT_INPUT_STATE_MESSAGE_NAME {
        return None;
    }

    let args = message.argument_list()?;
    let state = BrowserTextInputState {
        editable: args.bool(0) != 0,
    };
    log::info!(
        "[browser::text_input] browser_process state_message editable={}",
        state.editable
    );
    Some(state)
}

pub(crate) fn extract_text_input_debug_from_message(
    message: &mut ProcessMessage,
) -> Option<String> {
    if CefString::from(&message.name()).to_string() != TEXT_INPUT_DEBUG_MESSAGE_NAME {
        return None;
    }

    let args = message.argument_list()?;
    Some(CefString::from(&args.string(0)).to_string())
}

pub(crate) fn send_text_input_state(frame: &mut Frame, focused_node: Option<&Domnode>) -> bool {
    let Some(message) =
        process_message_create(Some(&CefString::from(TEXT_INPUT_STATE_MESSAGE_NAME)))
    else {
        return false;
    };

    let Some(args) = message.argument_list() else {
        return false;
    };

    let editable = focused_node.is_some_and(|node| node.is_editable() != 0);
    log::info!(
        "[browser::text_input] renderer_send editable={editable} frame_focused={}",
        frame.is_focused() != 0
    );
    args.set_bool(0, editable as i32);

    let mut message = message;
    frame.send_process_message(ProcessId::BROWSER, Some(&mut message));
    true
}

#[derive(Clone)]
pub(crate) struct TextInputDebugBridgeV8Handler;

wrap_v8_handler! {
    pub(crate) struct TextInputDebugBridgeV8HandlerBuilder {
        handler: TextInputDebugBridgeV8Handler,
    }

    impl V8Handler {
        fn execute(
            &self,
            _name: Option<&CefString>,
            _object: Option<&mut V8Value>,
            arguments: Option<&[Option<V8Value>]>,
            _retval: Option<&mut Option<V8Value>>,
            exception: Option<&mut CefString>,
        ) -> ::std::os::raw::c_int {
            let payload = arguments
                .and_then(|arguments| arguments.first())
                .and_then(|value| value.as_ref())
                .filter(|value| value.is_string() != 0)
                .map(|value| CefString::from(&value.string_value()).to_string())
                .unwrap_or_default();
            let Some(message) =
                process_message_create(Some(&CefString::from(TEXT_INPUT_DEBUG_MESSAGE_NAME)))
            else {
                let _ = exception;
                return 0;
            };

            let Some(args) = message.argument_list() else {
                let _ = exception;
                return 0;
            };

            args.set_string(0, Some(&CefString::from(payload.as_str())));

            let Some(context) = v8_context_get_current_context() else {
                let _ = exception;
                return 0;
            };

            let Some(frame) = context.frame() else {
                let _ = exception;
                return 0;
            };

            let mut message = message;
            frame.send_process_message(ProcessId::BROWSER, Some(&mut message));
            1
        }
    }
}

impl TextInputDebugBridgeV8HandlerBuilder {
    pub(crate) fn build() -> V8Handler {
        Self::new(TextInputDebugBridgeV8Handler)
    }
}

pub(crate) fn install_text_input_debug_bridge(_frame: &mut Frame, context: &mut V8Context) {
    let mut handler = TextInputDebugBridgeV8HandlerBuilder::build();
    let Some(mut bridge) = v8_value_create_function(
        Some(&CefString::from(TEXT_INPUT_DEBUG_BRIDGE_NAME)),
        Some(&mut handler),
    ) else {
        return;
    };

    let Some(global) = context.global() else {
        return;
    };

    global.set_value_bykey(
        Some(&CefString::from(TEXT_INPUT_DEBUG_BRIDGE_NAME)),
        Some(&mut bridge),
        Default::default(),
    );

    let mut result = None;
    let mut eval_exception = None::<cef::V8Exception>;
    if context.eval(
        Some(&CefString::from(TEXT_INPUT_DEBUG_OBSERVER_SCRIPT)),
        Some(&CefString::from("glass://text_input_debug.js")),
        0,
        Some(&mut result),
        Some(&mut eval_exception),
    ) == 0
        && let Some(eval_exception) = eval_exception
    {
        log::warn!(
            "[browser::text_input] Failed to install text input observer: {}",
            CefString::from(&eval_exception.message()).to_string()
        );
    }
}

pub(crate) fn key_down_dispatch(
    keystroke: &Keystroke,
    text_input_editable: bool,
    text_input_composing: bool,
) -> BrowserKeyDispatch {
    let dispatch = if keystroke.modifiers.platform
        || keystroke.modifiers.control
        || (keystroke.modifiers.function && keystroke.key_char.is_some())
    {
        BrowserKeyDispatch::App
    } else if should_use_text_input(keystroke, text_input_editable, text_input_composing) {
        BrowserKeyDispatch::TextInput
    } else {
        BrowserKeyDispatch::Browser
    };

    if keystroke.key_char.is_some()
        || matches!(dispatch, BrowserKeyDispatch::TextInput)
        || text_input_editable
    {
        log::info!(
            "[browser::text_input] key_down_dispatch key={} key_char={:?} editable={} composing={} route={dispatch:?}",
            keystroke.key,
            keystroke.key_char,
            text_input_editable,
            text_input_composing,
        );
    }

    dispatch
}

pub(crate) fn key_up_dispatch(
    keystroke: &Keystroke,
    text_input_editable: bool,
    text_input_composing: bool,
) -> BrowserKeyDispatch {
    let dispatch = if keystroke.modifiers.platform
        || keystroke.modifiers.control
        || (keystroke.modifiers.function && keystroke.key_char.is_some())
    {
        BrowserKeyDispatch::App
    } else if should_use_text_input(keystroke, text_input_editable, text_input_composing) {
        BrowserKeyDispatch::TextInput
    } else {
        BrowserKeyDispatch::Browser
    };

    if keystroke.key_char.is_some()
        || matches!(dispatch, BrowserKeyDispatch::TextInput)
        || text_input_editable
    {
        log::info!(
            "[browser::text_input] key_up_dispatch key={} key_char={:?} editable={} composing={} route={dispatch:?}",
            keystroke.key,
            keystroke.key_char,
            text_input_editable,
            text_input_composing,
        );
    }

    dispatch
}

fn should_use_text_input(
    keystroke: &Keystroke,
    text_input_editable: bool,
    text_input_composing: bool,
) -> bool {
    if text_input_composing {
        return true;
    }

    if !text_input_editable {
        return false;
    }

    if keystroke.key_char.is_some() {
        return true;
    }

    !matches!(
        keystroke.key.as_str(),
        "enter"
            | "backspace"
            | "tab"
            | "delete"
            | "escape"
            | "space"
            | "left"
            | "right"
            | "up"
            | "down"
            | "home"
            | "end"
            | "pageup"
            | "pagedown"
            | "f1"
            | "f2"
            | "f3"
            | "f4"
            | "f5"
            | "f6"
            | "f7"
            | "f8"
            | "f9"
            | "f10"
            | "f11"
            | "f12"
    )
}

#[cfg(test)]
mod tests {
    use super::{BrowserKeyDispatch, key_down_dispatch, key_up_dispatch};
    use gpui::{Keystroke, Modifiers};

    fn keystroke(key: &str, key_char: Option<&str>, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            key: key.into(),
            key_char: key_char.map(str::to_string),
            modifiers,
            native_key_code: None,
        }
    }

    #[test]
    fn printable_keys_use_browser_route_when_page_is_not_editable() {
        let keystroke = keystroke("e", Some("e"), Modifiers::default());

        assert_eq!(
            key_down_dispatch(&keystroke, false, false),
            BrowserKeyDispatch::Browser
        );
        assert_eq!(
            key_up_dispatch(&keystroke, false, false),
            BrowserKeyDispatch::Browser
        );
    }

    #[test]
    fn printable_keys_use_text_input_when_page_is_editable() {
        let keystroke = keystroke("e", Some("e"), Modifiers::default());

        assert_eq!(
            key_down_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::TextInput
        );
        assert_eq!(
            key_up_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::TextInput
        );
    }

    #[test]
    fn navigation_keys_still_reach_browser_when_page_is_editable() {
        let keystroke = keystroke("left", None, Modifiers::default());

        assert_eq!(
            key_down_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::Browser
        );
        assert_eq!(
            key_up_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::Browser
        );
    }

    #[test]
    fn command_shortcuts_stay_in_app_dispatch() {
        let keystroke = keystroke("c", Some("c"), Modifiers::command());

        assert_eq!(
            key_down_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::App
        );
        assert_eq!(
            key_up_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::App
        );
    }

    #[test]
    fn function_printable_shortcuts_stay_in_app_dispatch() {
        let keystroke = keystroke(
            "e",
            Some("e"),
            Modifiers {
                function: true,
                ..Modifiers::default()
            },
        );

        assert_eq!(
            key_down_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::App
        );
        assert_eq!(
            key_up_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::App
        );
    }

    #[test]
    fn composing_keys_stay_in_text_input_route() {
        let keystroke = keystroke("e", Some("e"), Modifiers::default());

        assert_eq!(
            key_down_dispatch(&keystroke, true, true),
            BrowserKeyDispatch::TextInput
        );
        assert_eq!(
            key_up_dispatch(&keystroke, true, true),
            BrowserKeyDispatch::TextInput
        );
    }

    #[test]
    fn dead_keys_stay_in_text_input_route_for_editable_fields() {
        let keystroke = keystroke("dead-acute", None, Modifiers::default());

        assert_eq!(
            key_down_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::TextInput
        );
        assert_eq!(
            key_up_dispatch(&keystroke, true, false),
            BrowserKeyDispatch::TextInput
        );
    }
}
