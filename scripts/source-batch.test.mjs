import test from 'node:test';
import assert from 'node:assert/strict';
import {sourceBatchLauncher} from '../src/source-batch.js';
import {limitTitle,weightedLength} from '../src/xianyu.js';

test('opens each source search once, including the next book, without reopening paused jobs', async () => {
  let requestId='one',state='waiting';const calls=[];
  const launch=sourceBatchLauncher({commerce:async()=>[{kind:'sources',state,result:{items:[{state:'searching',requestId,searchUrl:'https://s.taobao.com/'+requestId}]}}]},async(...args)=>calls.push(args));
  await launch();await launch();assert.equal(calls.length,1);
  requestId='two';await launch();assert.equal(calls.length,2);
  state='paused';requestId='three';await launch();assert.equal(calls.length,2);
});
test('title limiting counts Chinese as two and English as one',()=>{
  assert.equal(limitTitle('书'.repeat(40)),'书'.repeat(30));
  assert.equal(limitTitle('x'.repeat(80)),'x'.repeat(60));
  assert.ok(weightedLength(limitTitle('教材ABC'.repeat(20)))<=60);
});
