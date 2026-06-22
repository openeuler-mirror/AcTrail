export function chip(label, value) {
  if (value === undefined || value === null || value === '') {
    return null;
  }
  return { label, value: String(value) };
}

export function compactChips(chips) {
  return chips.filter(Boolean);
}

export function compactRows(rows) {
  return Object.entries(rows).filter(([, value]) => value !== undefined && value !== null && value !== '');
}

export function firstPresent(...values) {
  return values.find((value) => value !== undefined && value !== null && value !== '');
}

export function previewText(text, maxLen) {
  const normalized = String(text ?? '').replace(/\s+/g, ' ').trim();
  if (!normalized) {
    return '';
  }
  if (normalized.length <= maxLen) {
    return normalized;
  }
  return `${normalized.slice(0, maxLen - 3)}...`;
}
