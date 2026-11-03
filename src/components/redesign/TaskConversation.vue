<script setup>
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { api } from "../../api";

const props = defineProps({ job: Object, active: Boolean });
const books = computed(() => props.job.result?.items || []);
const bookId = ref(books.value[0]?.bookId);
const data = ref(null), loading = ref(false), error = ref("");
let timer, version = 0;
const running = computed(() => ["queued", "running"].includes(props.job.state));
const time = value => new Date(value).toLocaleString("zh-CN");
const fields = { analysis: "分析与建议", text: "回复", title: "商品标题", description: "商品描述", sellPrice: "建议售价（元）", referenceSourceId: "参考货源编号", recommendedSourceId: "推荐货源编号", backupSourceIds: "备用货源编号", sources: "货源比较" };
function executionStatus(run) {
  const execution = run.execution;
  if (!execution?.pid) return run.stdout?.bytes > 0 ? "已有输出，历史记录未保存进程号" : "未记录 CLI 启动结果";
  if (execution.status === "exited") return `已退出（退出码 ${execution.exitCode ?? '未知'}）`;
  return { running: "已记录启动，尚无退出记录", canceled: "已停止", timed_out: "超时停止", management_failed: "进程管理失败", wait_failed: "进程状态读取失败" }[execution.status] || execution.status;
}
function replyBlocks(text) {
  try {
    const clean = text.trim().replace(/^```(?:json)?\s*/, "").replace(/\s*```$/, "");
    const answer = JSON.parse(clean);
    if (answer && !Array.isArray(answer) && typeof answer === "object") {
      return Object.entries(answer).filter(([, value]) => value != null && value !== "").map(([key, value]) => ({
        label: fields[key] || key,
        text: typeof value === "string" ? value : JSON.stringify(value, null, 2),
      }));
    }
  } catch { /* Partial streaming replies are displayed as text. */ }
  return [{ label: "", text }];
}
async function refresh() {
  clearTimeout(timer);
  if (!props.active || !bookId.value) return;
  const current = ++version;
  loading.value = !data.value;
  try {
    const result = await api.commerce(`/jobs/${props.job.id}/conversation?bookId=${bookId.value}`);
    if (current !== version) return;
    data.value = result;
    error.value = "";
  } catch (e) {
    if (current === version) error.value = e.message;
  } finally {
    if (current === version) {
      loading.value = false;
      if (props.active && running.value) timer = setTimeout(() => document.hidden ? schedule() : refresh(), 2000);
    }
  }
}
function schedule() { timer = setTimeout(refresh, 2000); }
watch(bookId, () => { version++; data.value = null; error.value = ""; refresh(); });
watch(() => [props.active, props.job.state], () => {
  version++;
  clearTimeout(timer);
  refresh();
}, { immediate: true });
onBeforeUnmount(() => { version++; clearTimeout(timer); });
</script>

<template>
  <section class="task-conversation" aria-labelledby="task-conversation-title">
    <div class="module-bar">
      <h3 id="task-conversation-title">AI 对话</h3>
      <button class="btn small" :disabled="loading" @click="refresh">刷新对话</button>
    </div>
    <label v-if="books.length > 1" class="field conversation-book">
      <span>查看图书</span>
      <SearchSelect v-model="bookId" class="input">
        <option v-for="book in books" :key="book.bookId" :value="book.bookId">{{ book.title }}</option>
      </SearchSelect>
    </label>
    <p v-if="error" class="error-panel" role="alert">对话读取失败：{{ error }}。可点击“刷新对话”重试。</p>
    <p v-if="loading" class="muted" role="status">正在读取对话…</p>
    <p v-if="data?.truncated" class="muted">当前显示最近 20 次执行记录。</p>
    <template v-if="data && !data.runs.length">
      <div class="conversation-message">
        <h4>本次提示词</h4>
        <pre>{{ job.request.prompt }}</pre>
      </div>
      <p class="muted">{{ running ? '等待 AI 开始执行，回复会自动更新。' : '没有找到本次任务的对话记录；历史记录可能未保留。' }}</p>
    </template>
    <article v-for="(run, index) in data?.runs || []" :key="run.id" class="conversation-run">
      <p class="muted conversation-time">执行记录 {{ index + 1 }} · {{ time(run.createdAt) }}</p>
      <dl class="cli-record" aria-label="CLI 执行状态">
        <div><dt>CLI 调用</dt><dd>{{ executionStatus(run) }}</dd></div>
        <div v-if="run.execution?.pid"><dt>启动进程号</dt><dd>{{ run.execution.pid }}</dd></div>
        <div v-if="run.execution?.startedAt"><dt>启动时间</dt><dd>{{ time(run.execution.startedAt) }}</dd></div>
        <div v-if="run.execution?.finishedAt"><dt>退出时间</dt><dd>{{ time(run.execution.finishedAt) }}</dd></div>
        <div><dt>标准输出 / 错误输出</dt><dd>{{ run.stdout?.bytes ?? 0 }} / {{ run.stderr?.bytes ?? 0 }} 字节</dd></div>
        <div v-if="run.stdout?.updatedAt"><dt>输出更新时间</dt><dd>{{ time(run.stdout.updatedAt) }}</dd></div>
      </dl>
      <div class="conversation-message">
        <h4>本次提示词</h4>
        <pre>{{ run.prompt || '未记录提示词' }}</pre>
      </div>
      <details class="conversation-input">
        <summary>查看完整输入资料与系统要求</summary>
        <div v-if="run.system" class="conversation-message"><h4>系统要求</h4><pre>{{ run.system }}</pre></div>
        <div class="conversation-message"><h4>输入资料</h4><pre>{{ run.input || '未保留输入资料' }}</pre></div>
      </details>
      <div class="conversation-message conversation-reply">
        <h4>AI 回复</h4>
        <template v-if="run.reply">
          <div v-for="(block, part) in replyBlocks(run.reply)" :key="part" class="conversation-part">
            <p v-if="block.label" class="conversation-label">{{ block.label }}</p>
            <pre>{{ block.text }}</pre>
          </div>
        </template>
        <p v-else class="muted">{{ running && index === data.runs.length - 1 ? '等待 AI 返回正文，内容会自动更新。' : '本次执行没有返回可显示的回复。' }}</p>
      </div>
      <p v-if="run.error" class="error-panel" role="alert">{{ run.error }}</p>
    </article>
  </section>
</template>

<style scoped>
.task-conversation { margin-top: 24px; padding-top: 20px; border-top: 1px solid var(--line); min-width: 0; }
.cli-record { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 12px; padding: 16px 0; border-block: 1px solid var(--line); }
.cli-record dt { color: var(--muted); margin-bottom: 4px; }
.cli-record dd { margin: 0; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
.conversation-book { max-width: 480px; margin-bottom: 16px; }
.conversation-run + .conversation-run { margin-top: 24px; padding-top: 20px; border-top: 1px solid var(--line); }
.conversation-time { margin-bottom: 12px; }
.conversation-message { margin: 12px 0; }
.conversation-message h4 { font-size: 13px; margin-bottom: 8px; }
.conversation-message pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font: inherit; line-height: 1.7; }
.conversation-input { margin: 16px 0; }
.conversation-input summary { cursor: pointer; color: var(--accent); }
.conversation-input pre { max-height: 320px; overflow: auto; padding: 12px; background: var(--paper); }
.conversation-reply { padding: 16px; background: var(--paper); border-radius: 6px; }
.conversation-part + .conversation-part { margin-top: 16px; }
.conversation-label { color: var(--muted); font-size: 12px; margin-bottom: 4px; }
</style>
