export function compactPath(source: string, maxLength = 72): string {
  if (source.length <= maxLength) return source;
  if (maxLength <= 1) return "…".slice(0, maxLength);
  const headLength = Math.min(14, Math.max(1, Math.floor((maxLength - 1) / 3)));
  const tailLength = Math.max(0, maxLength - headLength - 1);
  const tail = tailLength > 0 ? source.slice(-tailLength) : "";
  return `${source.slice(0, headLength)}…${tail}`;
}
