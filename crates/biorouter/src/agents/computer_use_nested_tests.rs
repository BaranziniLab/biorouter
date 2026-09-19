mod computer_use_nested_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    #[derive(Default)]
    struct NativeNestedClient {
        calls: AtomicUsize,
        release: tokio::sync::Notify,
    }

    async fn fixture(
        allowed: Vec<String>,
    ) -> (TempDir, Arc<ExtensionManager>, Arc<NativeNestedClient>) {
        let (root, manager, _) = manager_bound_to(crate::privacy::ProviderTier::Public);
        let manager = Arc::new(manager);
        let client = Arc::new(NativeNestedClient::default());
        manager
            .add_mock_extension_with_tools("computercontroller".into(), client.clone(), allowed)
            .await;
        manager
            .add_extension(ExtensionConfig::Platform {
                name: "code_execution".into(),
                description: "test sandbox".into(),
                bundled: Some(true),
                available_tools: vec![],
            })
            .await
            .unwrap();
        (root, manager, client)
    }

    async fn execute(manager: &ExtensionManager, session: &str) -> super::super::ToolCallResult {
        manager.dispatch_tool_call(session, CallToolRequestParams {
            name: "code_execution__execute_code".into(),
            arguments: Some(serde_json::json!({"code": "import { get_app_state } from 'computercontroller'; get_app_state({app:'synthetic'});"}).as_object().unwrap().clone()),
            meta: None, task: None,
        }, crate::privacy::CallCapability::for_test_restricted(), CancellationToken::new()).await.unwrap()
    }

    #[tokio::test]
    async fn computer_use_nested_requires_approval_and_withholds_revoked_observation() {
        let _serial = crate::security::computer_use::tests::test_serial()
            .lock()
            .await;
        let (_root, manager, client) = fixture(vec![]).await;
        let _task = manager.computer_use.task_guard();
        manager
            .bind_computer_use_task("nested-native")
            .await
            .unwrap();
        let result = execute(&manager, "nested-native").await;
        let mut running = tokio::spawn(result.result);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if manager
                    .computer_use_status("nested-native")
                    .await
                    .unwrap()
                    .requested
                {
                    break;
                }
                assert!(
                    !running.is_finished(),
                    "nested call exited before reaching consent"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(client.calls.load(Ordering::SeqCst), 0);
        let status = manager.computer_use_status("nested-native").await.unwrap();
        manager
            .approve_computer_use("nested-native", &status.challenge_id)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while client.calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        manager.computer_use.revoke();
        client.release.notify_one();
        let result = tokio::time::timeout(Duration::from_secs(5), &mut running)
            .await
            .unwrap()
            .unwrap();
        let rendered = format!("{result:?}");
        assert!(!rendered.contains("PRIVATE_NATIVE_SENTINEL"));
        assert!(
            result.is_err() || result.unwrap().is_error == Some(true),
            "revoked nested call must fail"
        );
        assert_eq!(client.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn computer_use_nested_cannot_reintroduce_disabled_or_restricted_tool() {
        let _serial = crate::security::computer_use::tests::test_serial()
            .lock()
            .await;
        for disabled in [false, true] {
            let (_root, manager, client) = fixture(vec!["list_apps".into()]).await;
            if disabled {
                manager
                    .remove_extension("computercontroller")
                    .await
                    .unwrap();
            }
            let _task = manager.computer_use.task_guard();
            manager
                .bind_computer_use_task("restricted-native")
                .await
                .unwrap();
            let result = execute(&manager, "restricted-native").await;
            let result = tokio::time::timeout(Duration::from_secs(5), result.result)
                .await
                .unwrap();
            assert!(result.is_err() || result.unwrap().is_error == Some(true));
            assert_eq!(client.calls.load(Ordering::SeqCst), 0);
            assert!(
                !manager
                    .computer_use_status("restricted-native")
                    .await
                    .unwrap()
                    .requested
            );
        }
    }
    #[async_trait::async_trait]
    impl McpClientTrait for NativeNestedClient {
        fn get_info(&self) -> Option<&InitializeResult> {
            None
        }

        async fn list_resources(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListResourcesResult, Error> {
            Err(Error::TransportClosed)
        }

        async fn read_resource(
            &self,
            _uri: &str,
            _cancellation_token: CancellationToken,
        ) -> Result<ReadResourceResult, Error> {
            Err(Error::TransportClosed)
        }

        async fn list_tools(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListToolsResult, Error> {
            Ok(ListToolsResult {
                tools: vec![Tool::new("get_app_state", "Synthetic state", Arc::new(serde_json::json!({"type":"object", "properties":{"app":{"type":"string"}}, "required":["app"]}).as_object().unwrap().clone()))],
                next_cursor: None,
                meta: None,
            })
        }

        async fn call_tool(
            &self,
            _name: &str,
            _arguments: Option<JsonObject>,
            _meta: McpMeta,
            _cancellation_token: CancellationToken,
        ) -> Result<CallToolResult, Error> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.release.notified().await;
            Ok(CallToolResult {
                content: vec![rmcp::model::Content::text("PRIVATE_NATIVE_SENTINEL")],
                is_error: None,
                structured_content: None,
                meta: None,
            })
        }

        async fn list_prompts(
            &self,
            _next_cursor: Option<String>,
            _cancellation_token: CancellationToken,
        ) -> Result<ListPromptsResult, Error> {
            Err(Error::TransportClosed)
        }

        async fn get_prompt(
            &self,
            _name: &str,
            _arguments: Value,
            _cancellation_token: CancellationToken,
        ) -> Result<GetPromptResult, Error> {
            Err(Error::TransportClosed)
        }

        async fn subscribe(&self) -> mpsc::Receiver<ServerNotification> {
            mpsc::channel(1).1
        }
    }
}
