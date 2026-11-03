import test from 'node:test';
import assert from 'node:assert/strict';
import { drawCrop, moveCrop, resizeCrop } from '../src/image-crop.js';

test('each corner keeps the opposite corner fixed', () => {
  const rect = { x: 100, y: 80, w: 600, h: 800 };
  assert.deepEqual(resizeCrop(rect, 'nw', 50, 30, 1000, 1200), { x: 150, y: 110, w: 550, h: 770 });
  assert.deepEqual(resizeCrop(rect, 'ne', 50, 30, 1000, 1200), { x: 100, y: 110, w: 650, h: 770 });
  assert.deepEqual(resizeCrop(rect, 'sw', 50, 30, 1000, 1200), { x: 150, y: 80, w: 550, h: 830 });
  assert.deepEqual(resizeCrop(rect, 'se', 50, 30, 1000, 1200), { x: 100, y: 80, w: 650, h: 830 });
});

test('cropping supports more than 40 percent and clamps to image bounds', () => {
  assert.deepEqual(resizeCrop({ x: 0, y: 0, w: 1000, h: 800 }, 'nw', 800, 600, 1000, 800), { x: 800, y: 600, w: 200, h: 200 });
  const rect = { x: 100, y: 80, w: 600, h: 700 };
  assert.deepEqual(resizeCrop(rect, 'nw', -1000, -1000, 1000, 800), { x: 0, y: 0, w: 700, h: 780 });
  assert.deepEqual(resizeCrop(rect, 'se', 1000, 1000, 1000, 800), { x: 100, y: 80, w: 900, h: 720 });
  assert.deepEqual(resizeCrop(rect, 'nw', 1000, 1000, 1000, 800), { x: 692, y: 772, w: 8, h: 8 });
});

test('moving preserves dimensions and reverse drawing stays within the image', () => {
  assert.deepEqual(moveCrop({ x: 100, y: 80, w: 600, h: 700 }, 800, -100, 1000, 800), { x: 400, y: 0, w: 600, h: 700 });
  assert.deepEqual(drawCrop({ x: 900, y: 700 }, { x: -20, y: -30 }, 1000, 800), { x: 0, y: 0, w: 900, h: 700 });
  assert.deepEqual(drawCrop({ x: 1000, y: 800 }, { x: 1100, y: 900 }, 1000, 800), { x: 992, y: 792, w: 8, h: 8 });
});
