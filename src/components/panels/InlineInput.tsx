import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

const INVALID_CHARS = /[/\\:]/;

interface InlineInputProps {
	defaultValue?: string;
	onCommit: (value: string) => void;
	onCancel: () => void;
	className?: string;
}

export function InlineInput({
	defaultValue = "",
	onCommit,
	onCancel,
	className,
}: InlineInputProps) {
	const [value, setValue] = useState(defaultValue);
	const [error, setError] = useState<string | null>(null);
	const inputRef = useRef<HTMLInputElement>(null);
	const committedRef = useRef(false);

	useEffect(() => {
		inputRef.current?.focus();
		if (defaultValue) {
			const dotIndex = defaultValue.lastIndexOf(".");
			if (dotIndex > 0) {
				inputRef.current?.setSelectionRange(0, dotIndex);
			} else {
				inputRef.current?.select();
			}
		}
	}, [defaultValue]);

	const handleCommit = () => {
		if (committedRef.current) return;
		const trimmed = value.trim();
		if (!trimmed || INVALID_CHARS.test(trimmed)) return;
		committedRef.current = true;
		onCommit(trimmed);
	};

	const handleKeyDown = (e: React.KeyboardEvent) => {
		if (e.key === "Enter") {
			e.preventDefault();
			handleCommit();
		} else if (e.key === "Escape") {
			e.preventDefault();
			committedRef.current = true;
			onCancel();
		}
	};

	const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
		const newValue = e.target.value;
		setValue(newValue);
		if (INVALID_CHARS.test(newValue)) {
			setError("/, \\, : は使用できません");
		} else {
			setError(null);
		}
	};

	return (
		<div className={cn("flex flex-col", className)}>
			<input
				ref={inputRef}
				type="text"
				value={value}
				onChange={handleChange}
				onKeyDown={handleKeyDown}
				onBlur={handleCommit}
				className="h-[22px] px-1 text-sm bg-input border border-primary rounded-sm outline-none w-full"
			/>
			{error && (
				<span className="text-[10px] text-destructive px-1">{error}</span>
			)}
		</div>
	);
}
