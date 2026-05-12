import { ChevronDown, ChevronUp, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "@/components/ui/select";
import { Separator } from "@/components/ui/separator";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import type { ReduceStrategy, Step, StepMode } from "@/types/workflow";

const FACET_SLOTS = [
	"policy",
	"knowledge",
	"instruction",
	"output_contract",
] as const;

export type FacetSlot = (typeof FACET_SLOTS)[number];

export function StepEditor({
	step,
	index,
	totalSteps,
	allFacetKeys,
	allStepNames,
	onUpdate,
	onRemove,
	onMove,
}: {
	step: Step;
	index: number;
	totalSteps: number;
	allFacetKeys: Record<FacetSlot, string[]>;
	allStepNames: string[];
	onUpdate: (updater: (s: Step) => Step) => void;
	onRemove: () => void;
	onMove: (direction: "up" | "down") => void;
}) {
	const [expanded, setExpanded] = useState(false);

	if (step.parallel) {
		return (
			<div className="rounded-md border border-border p-3">
				<div className="flex items-center justify-between">
					<div className="flex items-center gap-2">
						<span className="text-xs text-muted-foreground">{index + 1}.</span>
						<span className="text-sm font-medium">{step.name}</span>
						<span className="rounded bg-blue-500/10 px-1.5 py-0.5 text-[10px] text-blue-500">
							parallel
						</span>
					</div>
					<span className="text-xs text-muted-foreground">
						(parallel blocks are edited via external editor)
					</span>
				</div>
			</div>
		);
	}

	return (
		<div className="rounded-md border border-border">
			<div className="flex items-center justify-between px-3 py-2">
				<button
					type="button"
					onClick={() => setExpanded(!expanded)}
					aria-expanded={expanded}
					className="flex items-center gap-2 flex-1 text-left"
				>
					<span className="text-xs text-muted-foreground">{index + 1}.</span>
					<span className="text-sm font-medium">{step.name}</span>
					{step.mode && (
						<span className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">
							{step.mode}
						</span>
					)}
				</button>
				<div className="flex items-center gap-0.5">
					<Button
						variant="ghost"
						size="icon"
						className="size-6"
						onClick={() => onMove("up")}
						disabled={index === 0}
						aria-label="Move step up"
					>
						<ChevronUp className="size-3" />
					</Button>
					<Button
						variant="ghost"
						size="icon"
						className="size-6"
						onClick={() => onMove("down")}
						disabled={index === totalSteps - 1}
						aria-label="Move step down"
					>
						<ChevronDown className="size-3" />
					</Button>
					<Button
						variant="ghost"
						size="icon"
						className="size-6 text-destructive hover:text-destructive"
						onClick={onRemove}
						aria-label="Remove step"
					>
						<Trash2 className="size-3" />
					</Button>
				</div>
			</div>

			{expanded && (
				<div className="px-3 pb-3 flex flex-col gap-3 text-xs">
					<Separator />

					{/* Name */}
					<div className="flex flex-col gap-1">
						<label
							htmlFor={`step-${index}-name`}
							className="font-medium text-muted-foreground"
						>
							Name
						</label>
						<Input
							id={`step-${index}-name`}
							value={step.name}
							onChange={(e) =>
								onUpdate((s) => ({ ...s, name: e.target.value }))
							}
							className="h-7 text-xs"
						/>
					</div>

					{/* Mode */}
					<div className="flex flex-col gap-1">
						<span className="font-medium text-muted-foreground">Mode</span>
						<Select
							value={step.mode ?? "auto"}
							onValueChange={(v) =>
								onUpdate((s) => ({ ...s, mode: v as StepMode }))
							}
						>
							<SelectTrigger className="h-7 text-xs">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="auto">Auto</SelectItem>
								<SelectItem value="approval">Approval</SelectItem>
							</SelectContent>
						</Select>
					</div>

					{/* Facet references */}
					{FACET_SLOTS.map((slot) => (
						<div key={slot} className="flex flex-col gap-1">
							<span className="font-medium text-muted-foreground capitalize">
								{slot === "output_contract" ? "Output Contract" : slot}
							</span>
							<Select
								value={step[slot] ?? "__none__"}
								onValueChange={(v) =>
									onUpdate((s) => ({
										...s,
										[slot]: v === "__none__" ? undefined : v,
									}))
								}
							>
								<SelectTrigger className="h-7 text-xs">
									<SelectValue placeholder="(none)" />
								</SelectTrigger>
								<SelectContent>
									<SelectItem value="__none__">(none)</SelectItem>
									{allFacetKeys[slot].map((k) => (
										<SelectItem key={k} value={k}>
											{k}
										</SelectItem>
									))}
								</SelectContent>
							</Select>
						</div>
					))}

					{/* Inline prompt */}
					<div className="flex flex-col gap-1">
						<label
							htmlFor={`step-${index}-prompt`}
							className="font-medium text-muted-foreground"
						>
							Inline Prompt
						</label>
						<Textarea
							id={`step-${index}-prompt`}
							value={step.inline_prompt ?? ""}
							onChange={(e) =>
								onUpdate((s) => ({
									...s,
									inline_prompt: e.target.value || undefined,
								}))
							}
							className="font-mono text-xs min-h-[60px]"
							rows={3}
							placeholder="(optional)"
						/>
					</div>

					{/* Transition Rules */}
					<div className="flex flex-col gap-1">
						<div className="flex items-center justify-between">
							<span className="font-medium text-muted-foreground">
								Transition Rules
							</span>
							<Button
								variant="ghost"
								size="icon"
								className="size-5"
								onClick={() =>
									onUpdate((s) => ({
										...s,
										rules: [...s.rules, { match: "", next: "" }],
									}))
								}
								aria-label="Add transition rule"
							>
								<Plus className="size-3" />
							</Button>
						</div>
						{step.rules.map((rule, ri) => (
							// biome-ignore lint/suspicious/noArrayIndexKey: rules have no stable unique id
							<div key={ri} className="flex items-center gap-1.5">
								<Input
									value={rule.match}
									onChange={(e) =>
										onUpdate((s) => ({
											...s,
											rules: s.rules.map((r, j) =>
												j === ri ? { ...r, match: e.target.value } : r,
											),
										}))
									}
									placeholder="match"
									className="h-7 text-xs flex-1"
								/>
								<span className="text-muted-foreground">→</span>
								<Select
									value={rule.next || "__empty__"}
									onValueChange={(v) =>
										onUpdate((s) => ({
											...s,
											rules: s.rules.map((r, j) =>
												j === ri
													? { ...r, next: v === "__empty__" ? "" : v }
													: r,
											),
										}))
									}
								>
									<SelectTrigger className="h-7 text-xs flex-1">
										<SelectValue placeholder="next step" />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="__empty__">(select)</SelectItem>
										{allStepNames
											.filter((n) => n !== step.name)
											.map((n) => (
												<SelectItem key={n} value={n}>
													{n}
												</SelectItem>
											))}
									</SelectContent>
								</Select>
								<Button
									variant="ghost"
									size="icon"
									className="size-5 text-destructive"
									onClick={() =>
										onUpdate((s) => ({
											...s,
											rules: s.rules.filter((_, j) => j !== ri),
										}))
									}
									aria-label="Remove transition rule"
								>
									<Trash2 className="size-3" />
								</Button>
							</div>
						))}
					</div>

					{/* Cycle Guard */}
					<div className="flex flex-col gap-1">
						<label
							htmlFor={`step-${index}-cycle`}
							className="font-medium text-muted-foreground"
						>
							Cycle Guard (max iterations)
						</label>
						<Input
							id={`step-${index}-cycle`}
							type="number"
							min={1}
							value={step.cycle_guard?.max_iterations ?? ""}
							onChange={(e) => {
								const val = e.target.value
									? Number.parseInt(e.target.value, 10)
									: undefined;
								onUpdate((s) => ({
									...s,
									cycle_guard: val ? { max_iterations: val } : undefined,
								}));
							}}
							className="h-7 text-xs w-24"
							placeholder="(none)"
						/>
					</div>

					{/* Pass previous response */}
					<div className="flex items-center gap-2">
						<input
							id={`step-${index}-pass-prev`}
							type="checkbox"
							checked={step.pass_previous_response ?? false}
							onChange={(e) =>
								onUpdate((s) => ({
									...s,
									pass_previous_response: e.target.checked || undefined,
								}))
							}
							className="size-3.5"
						/>
						<label
							htmlFor={`step-${index}-pass-prev`}
							className="font-medium text-muted-foreground text-xs"
						>
							Pass previous response
						</label>
					</div>

					{/* Pass output from */}
					<div className="flex flex-col gap-1">
						<span className="font-medium text-muted-foreground">
							Pass output from
						</span>
						<div className="flex flex-wrap gap-1">
							{allStepNames
								.filter((n) => n !== step.name)
								.map((n) => {
									const selected = step.pass_output_from?.includes(n) ?? false;
									return (
										<button
											key={n}
											type="button"
											onClick={() =>
												onUpdate((s) => {
													const current = s.pass_output_from ?? [];
													const next = selected
														? current.filter((x) => x !== n)
														: [...current, n];
													return {
														...s,
														pass_output_from:
															next.length > 0 ? next : undefined,
													};
												})
											}
											className={cn(
												"rounded px-1.5 py-0.5 text-[10px] border",
												selected
													? "bg-primary text-primary-foreground border-primary"
													: "border-border text-muted-foreground hover:bg-muted",
											)}
										>
											{n}
										</button>
									);
								})}
						</div>
					</div>

					{/* Collect / Reduce */}
					<div className="flex flex-col gap-1">
						<div className="flex items-center justify-between">
							<span className="font-medium text-muted-foreground">
								Collect / Reduce
							</span>
							{!step.collect ? (
								<Button
									variant="ghost"
									size="sm"
									className="h-5 text-[10px]"
									onClick={() =>
										onUpdate((s) => ({
											...s,
											collect: { from: [], reduce: "last" as ReduceStrategy },
										}))
									}
								>
									Enable
								</Button>
							) : (
								<Button
									variant="ghost"
									size="sm"
									className="h-5 text-[10px] text-destructive"
									onClick={() =>
										onUpdate((s) => ({
											...s,
											collect: undefined,
										}))
									}
								>
									Remove
								</Button>
							)}
						</div>
						{step.collect && (
							<div className="flex flex-col gap-1.5 ml-2">
								<div className="flex flex-col gap-1">
									<span className="text-muted-foreground">From:</span>
									<div className="flex flex-wrap gap-1">
										{allStepNames
											.filter((n) => n !== step.name)
											.map((n) => {
												const selected =
													step.collect?.from.includes(n) ?? false;
												return (
													<button
														key={n}
														type="button"
														onClick={() =>
															onUpdate((s) => {
																const current = s.collect?.from ?? [];
																const next = selected
																	? current.filter((x) => x !== n)
																	: [...current, n];
																const base = s.collect ?? {
																	from: [],
																	reduce: "last" as ReduceStrategy,
																};
																return {
																	...s,
																	collect: {
																		...base,
																		from: next,
																	},
																};
															})
														}
														className={cn(
															"rounded px-1.5 py-0.5 text-[10px] border",
															selected
																? "bg-primary text-primary-foreground border-primary"
																: "border-border text-muted-foreground hover:bg-muted",
														)}
													>
														{n}
													</button>
												);
											})}
									</div>
								</div>
								<div className="flex items-center gap-2">
									<span className="text-muted-foreground">Reduce:</span>
									<Select
										value={step.collect.reduce}
										onValueChange={(v) =>
											onUpdate((s) => {
												const base = s.collect ?? {
													from: [],
													reduce: "last" as ReduceStrategy,
												};
												return {
													...s,
													collect: {
														...base,
														reduce: v as ReduceStrategy,
													},
												};
											})
										}
									>
										<SelectTrigger className="h-7 text-xs w-36">
											<SelectValue />
										</SelectTrigger>
										<SelectContent>
											<SelectItem value="last">last</SelectItem>
											<SelectItem value="concat">concat</SelectItem>
											<SelectItem value="grouped">grouped</SelectItem>
											<SelectItem value="any_needs_fix">
												any_needs_fix
											</SelectItem>
											<SelectItem value="all_passed">all_passed</SelectItem>
										</SelectContent>
									</Select>
								</div>
							</div>
						)}
					</div>
				</div>
			)}
		</div>
	);
}
