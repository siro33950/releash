import {
	AlertTriangle,
	Ban,
	CheckCircle2,
	Circle,
	Clock,
	Loader2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { ProviderAgentSessionItem } from "@/types/provider-agent-session";
import type { WorkspaceNodeStatus } from "@/types/workspace-tree";

// standalone AgentSession行の実行状態表現。workflow node行と同じ
// 「アイコン色＝状態」の語彙に合わせる（open＋runningはrunning nodeと同じ
// blue＋pulse、open＋idleはニュートラル、paused異常終了はdestructive、
// paused正常とarchivedはqueued/abortedと同じ非活性のdim）。
export interface ProviderAgentSessionIconPresentation {
	className: string;
	pulse: boolean;
	statusLabel: string;
}

export function providerAgentSessionIconPresentation(
	session: Pick<
		ProviderAgentSessionItem,
		"lifecycle" | "activity" | "lastExitAbnormal"
	>,
): ProviderAgentSessionIconPresentation {
	if (session.lifecycle === "open") {
		if (session.activity === "running") {
			return {
				className: "text-blue-600 dark:text-blue-300",
				pulse: true,
				statusLabel: "running",
			};
		}
		return {
			className: "text-foreground",
			pulse: false,
			statusLabel: "open",
		};
	}
	if (session.lifecycle === "paused") {
		if (session.lastExitAbnormal) {
			return {
				className: "text-destructive",
				pulse: false,
				statusLabel: "paused (exited abnormally)",
			};
		}
		return {
			className: "text-muted-foreground",
			pulse: false,
			statusLabel: "paused",
		};
	}
	return {
		className: "text-muted-foreground",
		pulse: false,
		statusLabel: "archived",
	};
}

export const workflowNodeIconClasses: Record<WorkspaceNodeStatus, string> = {
	queued: "text-muted-foreground",
	running: "text-blue-600 dark:text-blue-300",
	failed: "text-red-600 dark:text-red-300",
	error: "text-destructive",
	waiting: "text-yellow-600 dark:text-yellow-300",
	interrupted: "text-orange-600 dark:text-orange-300",
	aborted: "text-muted-foreground",
	completed: "text-green-600 dark:text-green-300",
};

/** running / waiting のときにアイコンを pulse させるかの判定。 */
export function isWorkspaceNodePulseStatus(
	status: WorkspaceNodeStatus,
): boolean {
	return status === "running" || status === "waiting";
}

interface WorkflowNodeStatusIconProps {
	status: WorkspaceNodeStatus;
	containerClassName?: string;
	iconClassName?: string;
	circleClassName?: string;
}

export function WorkflowNodeStatusIcon({
	status,
	containerClassName,
	iconClassName = "size-3.5 shrink-0",
	circleClassName = "size-2.5 shrink-0",
}: WorkflowNodeStatusIconProps) {
	const colorClassName = workflowNodeIconClasses[status];
	const inheritedColor = containerClassName ? undefined : colorClassName;
	const baseIconClassName = cn(iconClassName, inheritedColor);
	const icon =
		status === "running" ? (
			<Loader2 className={cn(baseIconClassName, "animate-spin")} />
		) : status === "completed" ? (
			<CheckCircle2 className={baseIconClassName} />
		) : status === "failed" || status === "error" ? (
			<AlertTriangle className={baseIconClassName} />
		) : status === "waiting" ? (
			<Clock className={baseIconClassName} />
		) : status === "aborted" ? (
			<Ban className={baseIconClassName} />
		) : (
			<Circle className={cn(circleClassName, inheritedColor)} />
		);

	if (!containerClassName) {
		return icon;
	}

	return (
		<span className={cn(containerClassName, colorClassName)} title={status}>
			{icon}
		</span>
	);
}
