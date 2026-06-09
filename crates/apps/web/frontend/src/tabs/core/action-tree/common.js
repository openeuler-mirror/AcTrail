export function compactMeta(parts) {
  return parts.filter((part) => part !== undefined && part !== null && part !== '').join(' ');
}

export function compactRows(rows) {
  return Object.fromEntries(
    Object.entries(rows).filter(([, value]) => value !== undefined && value !== null && value !== ''),
  );
}

export function kindClass(kind) {
  return String(kind).replaceAll('.', '-').replaceAll(':', '-').replaceAll('_', '-');
}

export function shortTime(value) {
  if (!value) {
    return '';
  }
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleTimeString();
}
