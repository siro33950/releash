pub(crate) mod tests {
    use super::super::*;
    use crate::adaptor::presenter::client::CommandRequestEncode;

    fn parse_approve_workflow_node_args(
        args: &serde_json::Value,
    ) -> Result<ApprovalCommand, String> {
        use crate::adaptor::presenter::client as wire;
        let request = wire::CommandRequest::from_value(
            "approve_workflow_node",
            serde_json::json!({"args":args}),
        )?;
        let wire::command_request::Command::ApproveWorkflowNode(request) = request.command.unwrap()
        else {
            panic!("approve request");
        };
        request.args.unwrap().try_into()
    }

    #[test]
    fn approve_args_accept_optional_node_execution_address() {
        let args = parse_approve_workflow_node_args(&serde_json::json!({
            "executionId": "00000000-0000-0000-0000-000000000001",
            "nodeName": "review",
            "nodeExecutionId": "node-execution-review",
        }))
        .unwrap();

        assert_eq!(
            args.node_execution_id.as_deref(),
            Some("node-execution-review")
        );
    }

    #[test]
    fn approve_args_keep_single_name_fallback_when_address_is_omitted() {
        let args = parse_approve_workflow_node_args(&serde_json::json!({
            "executionId": "00000000-0000-0000-0000-000000000001",
            "nodeName": "review",
        }))
        .unwrap();

        assert!(args.node_execution_id.is_none());
    }
}
