export function sourceBatchLauncher(api, open) {
  const opened = new Set();
  let busy = false;
  return async () => {
    if (busy) return;
    busy = true;
    try {
      const jobs = await api.commerce('/jobs');
      for (const job of jobs.filter(j => j.kind === 'sources' && j.state === 'waiting')) {
        const item = job.result?.items?.find(i => i.state === 'searching');
        if (!item?.searchUrl || !item.requestId || opened.has(item.requestId)) continue;
        await open('taobao', item.searchUrl);
        opened.add(item.requestId);
        break;
      }
    } finally { busy = false; }
  };
}
