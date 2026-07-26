import { useApplicationShutdownSupervision } from "@/hooks/useOperationSupervision";

/**
 * S10: the single application quit flight. It is rendered at application scope
 * so a Session surface never presents another scope's failure or action.
 */
export function ApplicationShutdownBanner() {
	const supervision = useApplicationShutdownSupervision();
	const retryableTargets = supervision.state.shutdownTargets.filter((target) =>
		target.actions.includes("retry_same_effect"),
	);
	if (
		!supervision.state.shutdown &&
		!supervision.state.shutdownOutcomeUnknown &&
		retryableTargets.length === 0
	) {
		return null;
	}
	return (
		<div
			className="border-b border-border bg-muted/40 px-3 py-2 text-xs"
			data-testid="application-shutdown"
		>
			{supervision.state.shutdown && (
				<div className="flex items-center gap-2">
					<span>Application shutdown: {supervision.state.shutdown.phase}</span>
					{supervision.state.shutdown.actions.includes("retry_quit") && (
						<button
							type="button"
							className="rounded border border-border px-1.5 py-0.5"
							onClick={() => void supervision.retryQuit()}
						>
							Retry quit
						</button>
					)}
				</div>
			)}
			{supervision.state.shutdownOutcomeUnknown && (
				<div data-testid="shutdown-outcome-unknown">
					Application shutdown outcome unknown:{" "}
					{supervision.state.shutdownOutcomeUnknown.operation_id} —{" "}
					{supervision.state.shutdownOutcomeUnknown.intent.type} (
					{supervision.state.shutdownOutcomeUnknown.intent.code})
				</div>
			)}
			{retryableTargets.map((target) => (
				<div key={target.ordinal} className="flex items-center gap-2">
					<span>Shutdown target {target.kind} requires reconciliation</span>
					<button
						type="button"
						className="rounded border border-border px-1.5 py-0.5"
						onClick={() => void supervision.retryShutdownTarget(target)}
					>
						Retry same effect
					</button>
				</div>
			))}
		</div>
	);
}
