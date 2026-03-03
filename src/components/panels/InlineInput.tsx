import { useEffect, useRef, useState } from "react";
import { Input } from "@/components/ui/input";
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
		// Radix UIのContextMenuが閉じる際にフォーカスをトリガー要素に戻すため、
		// その復元処理の後にフォーカスを当てる必要がある
		const timerId = setTimeout(() => {
			inputRef.current?.focus();
			if (defaultValue) {
				const dotIndex = defaultValue.lastIndexOf(".");
				if (dotIndex > 0) {
					inputRef.current?.setSelectionRange(0, dotIndex);
				} else {
					inputRef.current?.select();
				}
			}
		}, 0);
		return () => clearTimeout(timerId);
	}, [defaultValue]);

	const handleCommit = () => {
		if (committedRef.current) return;
		const trimmed = value.trim();
		if (!trimmed || INVALID_CHARS.test(trimmed)) {
			committedRef.current = true;
			onCancel();
			return;
		}
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
			setError("/, \\, : are not allowed");
		} else {
			setError(null);
		}
	};

	return (
		<div className={cn("flex flex-col", className)}>
			<Input
				ref={inputRef}
				type="text"
				value={value}
				onChange={handleChange}
				onKeyDown={handleKeyDown}
				onBlur={handleCommit}
				onClick={(e) => e.stopPropagation()}
				className="h-[22px] px-1 text-sm bg-input border border-primary rounded-sm shadow-none outline-none focus-visible:ring-0 w-full"
			/>
			{error && (
				<span className="text-[10px] text-destructive px-1">{error}</span>
			)}
		</div>
	);
}
