biorouter::keep_this_test_binary_off_the_os_credential_store!();

use biorouter::providers::base::Provider;
use biorouter::providers::xai::XaiProvider;

#[test]
fn xai_chat_catalogue_excludes_image_generation_only_models() {
    let _guard = env_lock::lock_env([
        ("BIOROUTER_CONTEXT_LIMIT", None::<&str>),
        ("BIOROUTER_PREDEFINED_MODELS", None::<&str>),
    ]);
    let metadata = XaiProvider::metadata();

    for image_model in [
        "grok-imagine-image",
        "grok-imagine-image-2.0",
        "grok-imagine-image-quality",
    ] {
        assert!(
            metadata
                .known_models
                .iter()
                .all(|model| model.name != image_model),
            "{image_model} cannot answer through xAI's chat/completions adapter"
        );
    }
}

#[test]
fn xai_chat_catalogue_preserves_flagship_and_text_image_choices() {
    let _guard = env_lock::lock_env([
        ("BIOROUTER_CONTEXT_LIMIT", None::<&str>),
        ("BIOROUTER_PREDEFINED_MODELS", None::<&str>),
    ]);
    let metadata = XaiProvider::metadata();
    assert_eq!(metadata.default_model, "grok-4.7");
    assert_eq!(metadata.known_models[0].name, metadata.default_model);

    for (name, context_limit) in [
        ("grok-4.7", 500_000),
        ("grok-4.6", 500_000),
        ("grok-4.5", 500_000),
        ("grok-4.3", 1_000_000),
        ("grok-build-0.1", 256_000),
    ] {
        let model = metadata
            .known_models
            .iter()
            .find(|model| model.name == name)
            .unwrap_or_else(|| panic!("missing chat model {name}"));
        assert_eq!(model.context_limit, context_limit, "{name}");
        assert_eq!(model.supports_vision, Some(true), "{name}");
        assert_eq!(
            model.supported_input_mime_types,
            Some(vec![
                "image/png".to_string(),
                "image/jpeg".to_string(),
                "image/jpg".to_string(),
            ]),
            "{name} accepts PNG and JPEG images alongside text"
        );
    }
}
