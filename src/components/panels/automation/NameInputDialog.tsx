import { Loader2 } from "lucide-react";
import { type FormEvent, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";

export function NameInputDialog({
	open,
	onOpenChange,
	title,
	description,
	placeholder,
	onSubmit,
}: {
	open: boolean;
	onOpenChange: (v: boolean) => void;
	title: string;
	description: string;
	placeholder: string;
	onSubmit: (name: string) => Promise<{ ok: boolean; error?: string }>;
}) {
	const [value, setValue] = useState("");
	const [error, setError] = useState<string | null>(null);
	const [saving, setSaving] = useState(false);

	useEffect(() => {
		if (!open) {
			setValue("");
			setError(null);
		}
	}, [open]);

	const handleSubmit = async (e: FormEvent) => {
		e.preventDefault();
		if (!value.trim()) return;
		setSaving(true);
		setError(null);
		try {
			const result = await onSubmit(value.trim());
			if (result.ok) {
				setValue("");
				onOpenChange(false);
			} else {
				setError(result.error ?? "Unknown error");
			}
		} catch (e) {
			setError(e instanceof Error ? e.message : String(e));
		} finally {
			setSaving(false);
		}
	};

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="sm:max-w-md">
				<form onSubmit={handleSubmit}>
					<DialogHeader>
						<DialogTitle>{title}</DialogTitle>
						<DialogDescription>{description}</DialogDescription>
					</DialogHeader>
					<div className="flex flex-col gap-2 py-4">
						<Input
							value={value}
							onChange={(e) => setValue(e.target.value)}
							placeholder={placeholder}
							autoFocus
						/>
						{error && <p className="text-xs text-destructive">{error}</p>}
					</div>
					<DialogFooter>
						<Button
							type="button"
							variant="ghost"
							size="sm"
							onClick={() => onOpenChange(false)}
						>
							Cancel
						</Button>
						<Button type="submit" size="sm" disabled={!value.trim() || saving}>
							{saving ? (
								<Loader2 className="size-3.5 animate-spin" />
							) : (
								"Create"
							)}
						</Button>
					</DialogFooter>
				</form>
			</DialogContent>
		</Dialog>
	);
}
