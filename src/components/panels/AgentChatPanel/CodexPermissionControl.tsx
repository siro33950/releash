import { ChevronDown } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuRadioGroup,
	DropdownMenuRadioItem,
	DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { PermissionMode } from "@/types/session";

type CodexSandbox = "read-only" | "workspace" | "full-access";
type CodexApproval = "ask" | "never";

const SANDBOX_LABELS: Record<CodexSandbox, string> = {
	"read-only": "Read only",
	workspace: "Workspace",
	"full-access": "Full access",
};

export const CODEX_PERMISSION_CYCLE: PermissionMode[] = [
	"plan",
	"default",
	"acceptEdits",
	"bypassPermissions",
];

function sandboxFromMode(mode: PermissionMode): CodexSandbox {
	if (mode === "plan") return "read-only";
	if (mode === "bypassPermissions") return "full-access";
	return "workspace";
}

function approvalFromMode(mode: PermissionMode): CodexApproval {
	return mode === "default" ? "ask" : "never";
}

export function nextCodexPermissionMode(mode: PermissionMode): PermissionMode {
	const currentIndex = CODEX_PERMISSION_CYCLE.indexOf(mode);
	const nextIndex =
		currentIndex === -1
			? 0
			: (currentIndex + 1) % CODEX_PERMISSION_CYCLE.length;
	return CODEX_PERMISSION_CYCLE[nextIndex];
}

interface CodexPermissionControlProps {
	mode: PermissionMode;
	onModeChange: (mode: PermissionMode) => void;
	disabled: boolean;
}

export function CodexPermissionControl({
	mode,
	onModeChange,
	disabled,
}: CodexPermissionControlProps) {
	const sandbox = sandboxFromMode(mode);
	const approval = approvalFromMode(mode);
	const currentLabel = SANDBOX_LABELS[sandbox];
	const canSelectApproval = sandbox === "workspace";

	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="ghost"
					size="xs"
					disabled={disabled}
					data-testid="codex-permission-trigger"
					className="gap-1"
				>
					{currentLabel}
					<ChevronDown className="size-3" />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent side="top" align="start">
				<div className="px-2 py-1.5 text-[11px] font-medium text-muted-foreground">
					Sandbox
				</div>
				<DropdownMenuRadioGroup
					value={sandbox}
					onValueChange={(value) =>
						onModeChange(modeFromSandbox(value as CodexSandbox, mode))
					}
				>
					<DropdownMenuRadioItem value="read-only">
						Read only
					</DropdownMenuRadioItem>
					<DropdownMenuRadioItem value="workspace">
						Workspace
					</DropdownMenuRadioItem>
					<DropdownMenuRadioItem value="full-access">
						Full access
					</DropdownMenuRadioItem>
				</DropdownMenuRadioGroup>
				<div className="px-2 py-1.5 text-[11px] font-medium text-muted-foreground">
					Approval
				</div>
				<DropdownMenuRadioGroup
					value={approval}
					onValueChange={(value) =>
						onModeChange(modeFromApproval(value as CodexApproval))
					}
				>
					<DropdownMenuRadioItem value="ask" disabled={!canSelectApproval}>
						Ask
					</DropdownMenuRadioItem>
					<DropdownMenuRadioItem value="never" disabled={!canSelectApproval}>
						Never
					</DropdownMenuRadioItem>
				</DropdownMenuRadioGroup>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function modeFromSandbox(
	sandbox: CodexSandbox,
	currentMode: PermissionMode,
): PermissionMode {
	switch (sandbox) {
		case "read-only":
			return "plan";
		case "full-access":
			return "bypassPermissions";
		case "workspace":
			return approvalFromMode(currentMode) === "ask"
				? "default"
				: "acceptEdits";
	}
}

function modeFromApproval(approval: CodexApproval): PermissionMode {
	return approval === "ask" ? "default" : "acceptEdits";
}
