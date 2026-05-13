export const workflowStateClasses: Record<string, string> = {
	running: "border-blue-500/50 bg-blue-500/10 text-blue-700 dark:text-blue-300",
	completed:
		"border-green-500/50 bg-green-500/10 text-green-700 dark:text-green-300",
	failed: "border-red-500/50 bg-red-500/10 text-red-700 dark:text-red-300",
	waiting_approval:
		"border-yellow-500/50 bg-yellow-500/10 text-yellow-700 dark:text-yellow-300",
	aborted: "border-muted-foreground/40 bg-muted text-muted-foreground",
	pending: "border-border bg-background text-muted-foreground",
};
