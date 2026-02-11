import { describe, expect, it } from "vitest";
import { buildDataUrl, getMimeType, isImageFile } from "../imageUtils";

describe("isImageFile", () => {
	it.each([
		"photo.png",
		"image.jpg",
		"image.jpeg",
		"anim.gif",
		"pic.bmp",
		"icon.svg",
		"pic.webp",
		"favicon.ico",
		"scan.tiff",
		"scan.tif",
		"modern.avif",
		"apple.heic",
		"apple.heif",
	])("returns true for %s", (path) => {
		expect(isImageFile(path)).toBe(true);
	});

	it.each(["file.ts", "readme.md", "data.json", "style.css", "noext"])(
		"returns false for %s",
		(path) => {
			expect(isImageFile(path)).toBe(false);
		},
	);

	it("is case-insensitive", () => {
		expect(isImageFile("PHOTO.PNG")).toBe(true);
		expect(isImageFile("image.JPG")).toBe(true);
	});

	it("handles paths with directories", () => {
		expect(isImageFile("/Users/foo/bar/image.png")).toBe(true);
		expect(isImageFile("src/assets/logo.svg")).toBe(true);
	});
});

describe("getMimeType", () => {
	it("returns correct mime for known extensions", () => {
		expect(getMimeType("photo.png")).toBe("image/png");
		expect(getMimeType("photo.jpg")).toBe("image/jpeg");
		expect(getMimeType("photo.jpeg")).toBe("image/jpeg");
		expect(getMimeType("icon.svg")).toBe("image/svg+xml");
		expect(getMimeType("pic.webp")).toBe("image/webp");
		expect(getMimeType("favicon.ico")).toBe("image/x-icon");
		expect(getMimeType("scan.tiff")).toBe("image/tiff");
		expect(getMimeType("modern.avif")).toBe("image/avif");
	});

	it("returns octet-stream for unknown extensions", () => {
		expect(getMimeType("file.xyz")).toBe("application/octet-stream");
	});
});

describe("buildDataUrl", () => {
	it("constructs a data URL from base64 and mime", () => {
		const result = buildDataUrl("AQID", "image/png");
		expect(result).toBe("data:image/png;base64,AQID");
	});
});
