const clamp = (value, min, max) => Math.max(min, Math.min(max, value));

export function moveCrop(rect, dx, dy, width, height) {
  return { ...rect, x: Math.round(clamp(rect.x + dx, 0, width - rect.w)),
    y: Math.round(clamp(rect.y + dy, 0, height - rect.h)) };
}

export function resizeCrop(rect, corner, dx, dy, width, height) {
  const minW = Math.min(8, width), minH = Math.min(8, height);
  let left = rect.x, top = rect.y, right = rect.x + rect.w, bottom = rect.y + rect.h;
  if (corner.includes('w')) left = clamp(Math.round(left + dx), 0, right - minW);
  if (corner.includes('e')) right = clamp(Math.round(right + dx), left + minW, width);
  if (corner.includes('n')) top = clamp(Math.round(top + dy), 0, bottom - minH);
  if (corner.includes('s')) bottom = clamp(Math.round(bottom + dy), top + minH, height);
  return { x: left, y: top, w: right - left, h: bottom - top };
}

export function drawCrop(start, end, width, height) {
  const minW = Math.min(8, width), minH = Math.min(8, height);
  const x = clamp(Math.round(Math.min(start.x, end.x)), 0, width - minW);
  const y = clamp(Math.round(Math.min(start.y, end.y)), 0, height - minH);
  const right = clamp(Math.round(Math.max(start.x, end.x)), x + minW, width);
  const bottom = clamp(Math.round(Math.max(start.y, end.y)), y + minH, height);
  return { x, y, w: right - x, h: bottom - y };
}
