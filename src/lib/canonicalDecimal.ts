const CANONICAL_NONNEGATIVE_DECIMAL = /^(0|[1-9][0-9]*)$/;

export function isCanonicalDecimal(value: string): boolean {
	return CANONICAL_NONNEGATIVE_DECIMAL.test(value);
}

export function compareCanonicalDecimal(left: string, right: string): number {
	if (!isCanonicalDecimal(left) || !isCanonicalDecimal(right)) {
		throw new Error("expected canonical nonnegative decimal strings");
	}
	if (left.length !== right.length) {
		return left.length < right.length ? -1 : 1;
	}
	if (left === right) return 0;
	return left < right ? -1 : 1;
}

export function canonicalDecimalToDisplayNumber(value: string): number {
	if (!isCanonicalDecimal(value)) {
		throw new Error("expected a canonical nonnegative decimal string");
	}
	return Number(value);
}
