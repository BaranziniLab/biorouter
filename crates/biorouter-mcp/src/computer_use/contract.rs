use rmcp::model::Tool;
use serde_json::{json, Value};

pub const TOOL_NAMES: [&str; 10] = [
    "list_apps",
    "get_app_state",
    "click",
    "perform_secondary_action",
    "scroll",
    "drag",
    "type_text",
    "press_key",
    "set_value",
    "screen_capture",
];

pub fn tools() -> Vec<Tool> {
    let definitions = [
        ("list_apps", "List running desktop applications.", json!({}), vec![]),
        ("get_app_state", "Inspect a running application's current window, accessibility elements and screenshot. Inspect before acting and refresh after uncertain outcomes.", json!({"app":{"type":"string"},"text_limit":{"anyOf":[{"type":"integer","minimum":1},{"type":"string","enum":["max"]}]},"max_tree_nodes":{"type":"integer","minimum":1},"max_tree_depth":{"type":"integer","minimum":1}}), vec!["app"]),
        ("click", "Click a current element index or screenshot coordinates. Prefer accessibility elements. sky_click is unsupported.", json!({"app":{"type":"string"},"element_index":{"type":"string"},"x":{"type":"number"},"y":{"type":"number"},"click_count":{"type":"integer"},"mouse_button":{"type":"string","enum":["left","right","middle"]},"click_method":{"type":"string","enum":["auto","accessibility","app_post","sky_click","global"]}}), vec!["app"]),
        ("perform_secondary_action", "Invoke a named secondary accessibility action on an inspected element.", json!({"app":{"type":"string"},"element_index":{"type":"string"},"action":{"type":"string"}}), vec!["app","element_index","action"]),
        ("scroll", "Scroll an inspected element up, down, left or right by pages.", json!({"app":{"type":"string"},"element_index":{"type":"string"},"direction":{"type":"string"},"pages":{"type":"number"}}), vec!["app","element_index","direction"]),
        ("drag", "Drag between coordinates in the application's latest screenshot.", json!({"app":{"type":"string"},"from_x":{"type":"number"},"from_y":{"type":"number"},"to_x":{"type":"number"},"to_y":{"type":"number"}}), vec!["app","from_x","from_y","to_x","to_y"]),
        ("type_text", "Type literal text into the inspected application.", json!({"app":{"type":"string"},"text":{"type":"string"}}), vec!["app","text"]),
        ("press_key", "Press a keyboard key or combination in the inspected application.", json!({"app":{"type":"string"},"key":{"type":"string"}}), vec!["app","key"]),
        ("set_value", "Set an inspected accessibility element's value.", json!({"app":{"type":"string"},"element_index":{"type":"string"},"value":{"type":"string"}}), vec!["app","element_index","value"]),
        ("screen_capture", "Capture a display or matching window with the native helper. list_only inventories windows and displays without capturing an image.", json!({"display":{"type":"integer","minimum":0},"window_title":{"type":"string","minLength":1},"list_only":{"type":"boolean"}}), vec![]),
    ];
    definitions.into_iter().map(|(name, description, properties, required)| {
        let mut schema = json!({"type":"object","properties":properties,"additionalProperties":false});
        if !required.is_empty() { schema["required"] = json!(required); }
        serde_json::from_value(json!({"name":name,"description":description,"inputSchema":schema,"annotations":{"readOnlyHint":matches!(name,"list_apps"|"get_app_state"|"screen_capture"),"openWorldHint":true}})).expect("static computer-use tool contract")
    }).collect()
}

pub fn semantic_schema(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "description" | "title" | "default" | "$schema"
                    )
                })
                .map(|(key, value)| (key.clone(), semantic_schema(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(semantic_schema).collect()),
        _ => value.clone(),
    }
}
