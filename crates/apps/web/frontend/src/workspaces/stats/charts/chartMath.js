export function scalePoint(index, count, left, right) {
  if (count <= 1) {
    return (left + right) / 2;
  }
  return left + ((right - left) * index) / (count - 1);
}

export function scaleValue(value, max, top, bottom) {
  const fraction = Math.min(Math.max(Number(value ?? 0) / Math.max(1, max), 0), 1);
  return bottom - (bottom - top) * fraction;
}

export function pathForPoints(points) {
  return points
    .map((point, index) => `${index === 0 ? 'M' : 'L'} ${point.x.toFixed(2)} ${point.y.toFixed(2)}`)
    .join(' ');
}

export function smoothDensity(values, sampleCount = 48) {
  const numericValues = values.map((value) => Number(value ?? 0)).filter((value) => value >= 0);
  const maxValue = Math.max(1, ...numericValues);
  const bandwidth = Math.max(1, maxValue / 10);
  return Array.from({ length: sampleCount }, (_, index) => {
    const x = (maxValue * index) / (sampleCount - 1);
    const density = numericValues.reduce((total, value) => {
      const z = (x - value) / bandwidth;
      return total + Math.exp(-0.5 * z * z);
    }, 0);
    return { x, value: density };
  });
}

export function describeArc(cx, cy, radius, startAngle, endAngle) {
  const start = polarToCartesian(cx, cy, radius, endAngle);
  const end = polarToCartesian(cx, cy, radius, startAngle);
  const largeArcFlag = endAngle - startAngle <= 180 ? '0' : '1';
  return `M ${start.x} ${start.y} A ${radius} ${radius} 0 ${largeArcFlag} 0 ${end.x} ${end.y}`;
}

export function describeSlice(cx, cy, radius, startAngle, endAngle) {
  const arc = describeArc(cx, cy, radius, startAngle, endAngle);
  return `${arc} L ${cx} ${cy} Z`;
}

function polarToCartesian(cx, cy, radius, angleInDegrees) {
  const angleInRadians = ((angleInDegrees - 90) * Math.PI) / 180;
  return {
    x: cx + radius * Math.cos(angleInRadians),
    y: cy + radius * Math.sin(angleInRadians),
  };
}
