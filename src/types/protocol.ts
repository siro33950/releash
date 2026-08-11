export type ReviewActorKind = "human" | "agent";
type ReviewThreadState = "open" | "resolved";

interface ReviewActor {
	kind: ReviewActorKind;
	backendId?: string | null;
	model?: string | null;
	displayName: string;
}

interface ReviewTarget {
	filePath?: string | null;
	lineNumber?: number | null;
	endLine?: number | null;
}

interface ReviewComment {
	id: string;
	threadId: string;
	author: ReviewActor;
	content: string;
	createdAt: number;
}

interface ReviewResolveInfo {
	actor: ReviewActor;
	outcome: string;
	summary: string;
	resolvedAt: number;
}

export interface ReviewThread {
	id: string;
	worktreeName: string;
	author: ReviewActor;
	target: ReviewTarget;
	state: ReviewThreadState;
	comments: ReviewComment[];
	resolve?: ReviewResolveInfo | null;
	createdAt: number;
	updatedAt: number;
	version: number;
	canResolve: boolean;
}
