import { invoke } from "@tauri-apps/api/core";
import { useCallback, useState } from "react";

export interface StepApprovalActionContext {
	worktreePath: string;
	executionId: string;
	stepName: string;
}

export function useStepApprovalAction({
	worktreePath,
	executionId,
	stepName,
}: StepApprovalActionContext) {
	const [rejectMode, setRejectMode] = useState(false);
	const [rejectComment, setRejectComment] = useState("");
	const [approvalError, setApprovalError] = useState<string | null>(null);

	const identityKey = `${worktreePath}|${executionId}|${stepName}`;
	const [prevIdentityKey, setPrevIdentityKey] = useState(identityKey);
	if (prevIdentityKey !== identityKey) {
		setPrevIdentityKey(identityKey);
		setRejectMode(false);
		setRejectComment("");
		setApprovalError(null);
	}

	const approve = useCallback(() => {
		return invoke("approve_workflow_step", {
			worktreePath,
			decision: "approve",
			executionId,
			stepName,
		})
			.then(() => setApprovalError(null))
			.catch((e) => {
				console.warn("[useStepApprovalAction] approve_workflow_step failed", e);
				setApprovalError(formatWorkflowApprovalError(e));
			});
	}, [worktreePath, executionId, stepName]);

	const openReject = useCallback(() => {
		setRejectMode(true);
		setApprovalError(null);
	}, []);

	const cancelReject = useCallback(() => {
		setRejectMode(false);
		setRejectComment("");
	}, []);

	const canSubmitReject = rejectComment.trim().length > 0;

	const submitReject = useCallback(() => {
		if (!canSubmitReject) {
			return Promise.resolve();
		}
		return invoke("approve_workflow_step", {
			worktreePath,
			decision: { reject: { comment: rejectComment } },
			executionId,
			stepName,
		})
			.then(() => {
				setRejectMode(false);
				setRejectComment("");
				setApprovalError(null);
			})
			.catch((e) => {
				console.warn("[useStepApprovalAction] approve_workflow_step failed", e);
				setApprovalError(formatWorkflowApprovalError(e));
			});
	}, [worktreePath, executionId, stepName, rejectComment, canSubmitReject]);

	return {
		rejectMode,
		rejectComment,
		setRejectComment,
		canSubmitReject,
		approvalError,
		approve,
		openReject,
		cancelReject,
		submitReject,
	};
}

function formatWorkflowApprovalError(error: unknown): string {
	const message = String(error);
	if (message.startsWith("invalid_state:")) {
		return "Workflow approval is no longer available for the current step.";
	}
	if (message.startsWith("validation_error:")) {
		return message.replace(/^validation_error:\s*/, "");
	}
	if (message.startsWith("unauthorized_worktree:")) {
		return "This approval belongs to a different worktree.";
	}
	if (message.startsWith("unauthorized_approval_target:")) {
		return "This approval request no longer matches the current workflow step.";
	}
	return message;
}
