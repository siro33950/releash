import { cva, type VariantProps } from "class-variance-authority";
import type * as React from "react";

import { cn } from "@/lib/utils";

const inputVariants = cva(
	"file:text-foreground placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground w-full min-w-0 rounded-md border bg-transparent shadow-xs transition-[color,box-shadow] outline-none file:inline-flex file:h-7 file:border-0 file:bg-transparent file:text-sm file:font-medium disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40 aria-invalid:border-destructive",
	{
		variants: {
			variant: {
				default:
					"dark:bg-input/30 border-input focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]",
				panel:
					"bg-muted border-border focus:outline-none focus:ring-1 focus:ring-primary",
			},
			size: {
				default: "h-9 px-3 py-1 text-base md:text-sm",
				sm: "h-7 px-2 py-1 text-xs",
				xs: "h-6 px-2 py-1 text-[10px]",
			},
		},
		defaultVariants: {
			variant: "default",
			size: "default",
		},
	},
);

type InputVariantProps = VariantProps<typeof inputVariants>;

function Input({
	className,
	type,
	variant,
	size,
	...props
}: Omit<React.ComponentProps<"input">, "size"> & InputVariantProps) {
	return (
		<input
			type={type}
			data-slot="input"
			className={cn(inputVariants({ variant, size, className }))}
			{...props}
		/>
	);
}

export { Input, inputVariants };
