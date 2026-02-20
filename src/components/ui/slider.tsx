import { Slider as SliderPrimitive } from "radix-ui";
import type * as React from "react";

import { cn } from "@/lib/utils";

function Slider({
	className,
	defaultValue,
	value,
	min = 0,
	max = 100,
	...props
}: React.ComponentProps<typeof SliderPrimitive.Root>) {
	const _value = value ?? defaultValue ?? [min];

	return (
		<SliderPrimitive.Root
			data-slot="slider"
			defaultValue={defaultValue}
			value={value}
			min={min}
			max={max}
			className={cn(
				"relative flex w-full touch-none items-center select-none data-[disabled]:opacity-50",
				className,
			)}
			{...props}
		>
			<SliderPrimitive.Track
				data-slot="slider-track"
				className="bg-muted relative h-1.5 w-full grow overflow-hidden rounded-full"
			>
				<SliderPrimitive.Range
					data-slot="slider-range"
					className="bg-primary absolute h-full"
				/>
			</SliderPrimitive.Track>
			{Array.from({ length: _value.length }, (_, index) => (
				<SliderPrimitive.Thumb
					data-slot="slider-thumb"
					// biome-ignore lint/suspicious/noArrayIndexKey: slider thumbs are positional and never reordered
					key={index}
					className="border-primary/50 bg-background focus-visible:ring-ring/50 block size-4 rounded-full border shadow-sm transition-colors outline-none focus-visible:ring-[3px] disabled:pointer-events-none"
				/>
			))}
		</SliderPrimitive.Root>
	);
}

export { Slider };
