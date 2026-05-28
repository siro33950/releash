import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { DiffCommentBody } from "../DiffCommentBody";

describe("DiffCommentBody", () => {
	it("renders bold markdown as <strong>", () => {
		render(<DiffCommentBody content="**bold text**" />);
		const strong = screen.getByText("bold text");
		expect(strong.tagName).toBe("STRONG");
	});

	it("renders inline code as <code>", () => {
		render(<DiffCommentBody content="use `foo()` here" />);
		const code = screen.getByText("foo()");
		expect(code.tagName).toBe("CODE");
	});

	it("renders unordered lists", () => {
		render(<DiffCommentBody content={"- one\n- two"} />);
		const items = screen.getAllByRole("listitem");
		expect(items).toHaveLength(2);
		expect(items[0]).toHaveTextContent("one");
		expect(items[1]).toHaveTextContent("two");
	});

	it("renders links with href", () => {
		render(<DiffCommentBody content="[example](https://example.com)" />);
		const link = screen.getByRole("link");
		expect(link).toHaveAttribute("href", "https://example.com");
		expect(link).toHaveTextContent("example");
	});

	it("renders GFM tables", () => {
		const table = `| Header | Value |
| ------ | ----- |
| A      | 1     |`;
		render(<DiffCommentBody content={table} />);
		expect(screen.getByText("Header")).toBeInTheDocument();
		expect(screen.getByText("Value")).toBeInTheDocument();
	});

	it("sanitizes script tags", () => {
		const { container } = render(
			<DiffCommentBody content={"<script>window.__pwn=true</script>safe"} />,
		);
		expect(container.querySelector("script")).toBeNull();
		expect(container).toHaveTextContent("safe");
	});

	it("applies markdown-preview-comment class for compact styling", () => {
		render(<DiffCommentBody content="hello" />);
		const node = screen.getByTestId("diff-comment-body");
		expect(node.className).toContain("markdown-preview");
		expect(node.className).toContain("markdown-preview-comment");
	});

	it("merges additional className via cn()", () => {
		render(<DiffCommentBody content="hello" className="mt-1" />);
		const node = screen.getByTestId("diff-comment-body");
		expect(node.className).toContain("mt-1");
	});
});
