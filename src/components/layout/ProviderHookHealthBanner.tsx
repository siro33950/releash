import { AlertTriangle } from "lucide-react";
import { useStateSubscriptionResult } from "@/hooks/useStateSubscription";

function providerLabel(provider: string): string {
	return `${provider.charAt(0).toUpperCase()}${provider.slice(1)}`;
}

export function ProviderHookHealthBanner() {
	const subscription = useStateSubscriptionResult("provider-hook-health");
	const warnings = subscription.value?.warnings ?? [];
	const readErrors = subscription.value?.readErrors ?? [];
	if (warnings.length === 0 && readErrors.length === 0 && !subscription.error)
		return null;
	const providers = [
		...new Set(warnings.map(({ provider }) => providerLabel(provider))),
	];

	return (
		<div
			role="alert"
			className="flex items-center gap-2 border-b border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-900 dark:text-amber-200"
		>
			<AlertTriangle className="size-4 shrink-0" />
			{warnings.length > 0 && (
				<span>
					Provider Hook health warning: {providers.join(", ")}. AgentSession
					operation remains available.
				</span>
			)}
			{readErrors.length > 0 && <span>{readErrors.join("; ")}</span>}
			{subscription.error && (
				<span>Provider Hook health read failed: {subscription.error}</span>
			)}
		</div>
	);
}
