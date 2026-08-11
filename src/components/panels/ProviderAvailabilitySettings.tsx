import { Loader2, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import type { useProviderAvailabilitySettings } from "@/hooks/useProviderAvailabilitySettings";

export function ProviderAvailabilitySettings({
	settings,
}: {
	settings: ReturnType<typeof useProviderAvailabilitySettings>;
}) {
	return (
		<div className="flex flex-col gap-3 rounded border p-3">
			<div className="flex items-center justify-between gap-2">
				<div>
					<p className="text-xs font-medium">Provider CLI availability</p>
					<p className="text-[10px] text-muted-foreground">
						Used by new AgentSessions and Workflow nodes.
					</p>
				</div>
				<Button
					type="button"
					variant="outline"
					size="sm"
					disabled={settings.loading || settings.refreshing || settings.saving}
					onClick={settings.refresh}
					aria-label="Refresh Provider CLI availability"
				>
					{settings.refreshing ? (
						<Loader2 className="size-3.5 animate-spin" />
					) : (
						<RefreshCw className="size-3.5" />
					)}
					Refresh
				</Button>
			</div>

			{settings.loading ? (
				<div className="flex justify-center py-4">
					<Loader2 className="size-4 animate-spin text-muted-foreground" />
				</div>
			) : (
				settings.providers.map((provider) => (
					<div
						key={provider.provider}
						className="flex flex-col gap-2 rounded bg-muted/40 p-2"
					>
						<div className="flex items-center justify-between gap-2">
							<div className="text-xs font-medium">{provider.displayName}</div>
							<div
								className={
									provider.available
										? "text-[10px] text-green-500"
										: "text-[10px] text-destructive"
								}
							>
								{provider.available ? "Available" : "Unavailable"}
							</div>
						</div>
						<div className="grid grid-cols-[auto_1fr] gap-x-2 text-[10px]">
							<span className="text-muted-foreground">Provider ID</span>
							<span className="truncate font-mono">{provider.provider}</span>
							<span className="text-muted-foreground">Default</span>
							<span className="truncate font-mono">
								{provider.defaultExecutable}
							</span>
							<span className="text-muted-foreground">Effective</span>
							<span className="truncate font-mono">
								{provider.effectiveExecutable}
							</span>
							<span className="text-muted-foreground">
								{provider.available ? "Resolved" : "Reason"}
							</span>
							<span className="truncate font-mono">
								{provider.resolvedExecutable ?? provider.unavailableReason}
							</span>
						</div>
						<div className="flex items-end gap-2">
							<div className="flex-1">
								<label
									htmlFor={`provider-executable-${provider.provider}`}
									className="text-[10px] text-muted-foreground"
								>
									{provider.displayName} executable override
								</label>
								<Input
									id={`provider-executable-${provider.provider}`}
									value={settings.drafts[provider.provider] ?? ""}
									placeholder={provider.defaultExecutable}
									disabled={settings.saving}
									onChange={(event) =>
										settings.setExecutable(
											provider.provider,
											event.target.value,
										)
									}
									className="h-8 font-mono text-xs"
								/>
							</div>
							<Button
								type="button"
								variant="outline"
								size="sm"
								disabled={
									settings.saving || provider.configuredExecutable === null
								}
								onClick={() => settings.reset(provider.provider)}
								aria-label={`Reset ${provider.displayName} executable`}
							>
								Reset
							</Button>
						</div>
					</div>
				))
			)}
			{settings.error && (
				<p role="alert" className="text-[10px] text-destructive">
					{settings.error}
				</p>
			)}
		</div>
	);
}
