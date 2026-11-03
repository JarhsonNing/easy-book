<script setup>
import {
  computed,
  onBeforeUnmount,
  reactive,
  ref,
  watch,
} from "vue";
import { api } from "../../api";
import TaskConversation from "./TaskConversation.vue";
const props = defineProps({ active: Boolean });
const emit = defineEmits(["open-book", "work-item", "open-schools", "open-draft"]);
const jobs = ref([]),
  adapters = ref([]),
  selected = ref(null),
  creating = ref(false),
  error = ref(""),
  refreshError = ref(""),
  detailError = ref(""),
  loading = ref(false),
  checkingId = ref(null),
  syncedAt = ref(""),
  checkedAt = ref(""),
  taskNumber = ref(""),
  page = ref(1),
  total = ref(0),
  counts = ref({}),
  busy = ref(false),
  filter = ref("");
const pageSize = 30;
const pages = computed(() => Math.max(1, Math.ceil(total.value / pageSize)));
const form = reactive({
  plugin: "",
  year: new Date().getFullYear() + 1,
  args: {},
  refresh: false,
});
const adapter = computed(() =>
  adapters.value.find((a) => a.ref === form.plugin),
);
const labels = {
  queued: "排队中",
  running: "运行中",
  waiting: "待处理",
  paused: "已暂停",
  completed: "已完成",
  partial: "部分失败",
  failed: "失败",
  canceled: "已取消",
};
let timer,
  run = 0,
  detailRun = 0,
  polling = 0;
async function refreshDetail(id) {
  const version = ++detailRun;
  checkingId.value = id;
  try {
    const job = await api.commerce(`/jobs/${id}`);
    if (version !== detailRun || selected.value?.id !== id) return;
    selected.value = job;
    checkedAt.value = new Date().toISOString();
    detailError.value = "";
  } catch (e) {
    if (version === detailRun && selected.value?.id === id) detailError.value = `状态核查失败：${e.message}，请重试。`;
  } finally {
    if (version === detailRun) checkingId.value = null;
  }
}
async function refresh() {
  const version = ++run;
  loading.value = true;
  const detail = selected.value && !busy.value && checkingId.value === null ? refreshDetail(selected.value.id) : Promise.resolve();
  try {
    const query = new URLSearchParams({ state: filter.value, page: page.value, pageSize });
    const result = await api.commerce("/jobs/page?" + query);
    if (version !== run) return;
    jobs.value = result.items;
    total.value = result.total;
    counts.value = result.counts;
    page.value = result.page;
    syncedAt.value = new Date().toISOString();
    refreshError.value = "";
  } catch (e) {
    if (version === run) refreshError.value = `任务同步失败：${e.message}。当前保留上次结果，请刷新重试。`;
  } finally {
    await detail;
    if (version === run) loading.value = false;
  }
}
async function newTask() {
  error.value = "";
  try {
    const r = await api.commerce("/catalog/adapters");
    adapters.value = r.adapters.filter((a) => !a.error);
    form.plugin = adapters.value[0]?.ref || "";
    form.args = {};
    creating.value = true;
  } catch (e) {
    error.value = e.message;
  }
}
watch(
  () => form.plugin,
  () => {
    form.args = {};
    if (adapter.value?.key === "ecnu") form.args.school = "华东师范大学";
  },
);
async function create() {
  busy.value = true;
  error.value = "";
  try {
    const j = await api.commerce("/jobs", {
      kind: "catalog",
      ...form,
      title:
        (form.args.school || adapter.value?.name || "学校目录") +
        " · " +
        form.year,
    });
    creating.value = false;
    selected.value = j;
    await refresh();
  } catch (e) {
    error.value = e.message;
  } finally {
    busy.value = false;
  }
}
async function control(j, action, body = {}) {
  busy.value = true;
  detailRun++;
  checkingId.value = null;
  error.value = "";
  try {
    selected.value = await api.commerce(`/jobs/${j.id}/${action}`, body);
    checkedAt.value = new Date().toISOString();
    detailError.value = "";
    await refresh();
  } catch (e) {
    error.value = e.message;
  } finally {
    busy.value = false;
  }
}
async function selectJob(job) {
  selected.value = job;
  checkedAt.value = "";
  detailError.value = "";
  await refreshDetail(job.id);
}
async function openJob(id) {
  error.value = "";
  const version = ++detailRun;
  checkingId.value = id;
  try {
    const job = await api.commerce('/jobs/' + id);
    if (version !== detailRun) return;
    selected.value = job;
    checkedAt.value = new Date().toISOString();
    detailError.value = "";
  } catch (e) {
    if (version === detailRun) error.value = `任务 #${id} 核查失败：${e.message}`;
  } finally {
    if (version === detailRun) checkingId.value = null;
  }
}
function lookup() {
  const id = Number(taskNumber.value);
  if (!Number.isSafeInteger(id) || id < 1) { error.value = "请输入有效的任务号"; return; }
  openJob(id);
}
function closeDetail() {
  detailRun++;
  selected.value = null;
  checkingId.value = null;
  detailError.value = "";
}
function openResult(event, ...args) {
  closeDetail();
  emit(event, ...args);
}
async function tick() {
  const version = ++polling;
  clearTimeout(timer);
  if (!props.active) return;
  await refresh();
  if (version !== polling) return;
  const poll = async () => {
    if (version !== polling || !props.active) return;
    if (!document.hidden) await refresh();
    if (version === polling) timer = setTimeout(poll, 2000);
  };
  timer = setTimeout(poll, 2000);
}
watch(filter, () => { page.value = 1; });
watch([filter, page], refresh);
watch(() => props.active, () => { if (!props.active) closeDetail(); tick(); }, { immediate: true });
let previousOverflow;
function restoreScroll() {
  if (previousOverflow !== undefined) {
    document.body.style.overflow = previousOverflow;
    previousOverflow = undefined;
  }
}
watch(() => Boolean(selected.value && props.active), (open) => {
  if (open) { previousOverflow = document.body.style.overflow; document.body.style.overflow = "hidden"; }
  else restoreScroll();
});
onBeforeUnmount(() => {
  clearTimeout(timer);
  run++;
  detailRun++;
  polling++;
  restoreScroll();
});
defineExpose({ refresh, openJob });
const time = (v) =>
  new Date(v).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
const fullTime = (v) => v ? new Date(v).toLocaleString("zh-CN", { hour12: false }) : "—";
</script>
<template>
  <section>
    <div class="module-bar task-toolbar">
      <label class="row task-filter">
        <span>状态</span>
        <select v-model="filter" class="input" aria-label="任务状态">
          <option value="">全部状态（{{ counts.all ?? 0 }}）</option>
          <option v-for="(label, state) in labels" :key="state" :value="state">{{ label }}（{{ counts[state] ?? 0 }}）</option>
        </select>
      </label>
      <form class="row task-lookup" @submit.prevent="lookup">
        <label for="task-number">任务号</label>
        <input id="task-number" v-model="taskNumber" class="input" type="number" min="1" step="1" placeholder="例如 4245" required />
        <button class="btn" :disabled="checkingId !== null">按号核查</button>
      </form>
      <button class="btn primary" @click="newTask">新建任务</button>
    </div>
    <div class="row task-sync">
      <span class="muted">共 {{ total }} 条{{ filter ? '“' + labels[filter] + '”' : '' }}任务<span v-if="syncedAt"> · 最近同步 {{ fullTime(syncedAt) }} · 每 2 秒自动更新</span></span>
      <button class="btn small" :disabled="loading" @click="refresh">{{ loading ? '正在同步…' : '刷新状态' }}</button>
    </div>
    <p v-if="refreshError" class="error-panel" role="alert">{{ refreshError }}</p>
    <p v-if="error" class="error-panel" role="alert">{{ error }}</p>
    <div class="data-surface">
      <table class="data-table task-table" :aria-busy="loading">
        <thead>
          <tr>
            <th scope="col">任务号</th>
            <th>任务</th>
            <th>阶段</th>
            <th>进度</th>
            <th>成功 / 失败</th>
            <th>状态</th>
            <th>更新时间</th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="j in jobs" :key="j.id">
            <td class="task-number">#{{ j.id }}</td>
            <td>
              <button class="text-button" @click="selectJob(j)">
                {{ j.title }}
              </button>
            </td>
            <td>{{ j.stage || "等待执行" }}</td>
            <td>
              {{ j.processed }}{{ j.total != null ? " / " + j.total : " 页" }}
            </td>
            <td>
              {{
                j.result?.programs ??
                j.result?.items?.filter((i) => i.state === "done").length ??
                "—"
              }}
              / {{ j.result?.errors?.length ?? j.result?.items?.filter(i=>i.state==='failed').length ?? (j.state === "failed" ? 1 : 0) }}
            </td>
            <td>
              <span
                class="tag"
                :class="
                  j.state === 'completed'
                    ? 'success'
                    : ['failed', 'partial'].includes(j.state)
                      ? 'warning'
                      : 'neutral'
                "
                >{{ labels[j.state] || j.state }}</span
              >
            </td>
            <td class="muted">{{ time(j.updatedAt) }}</td>
            <td>
              <button class="btn small" :disabled="checkingId !== null" @click="selectJob(j)">{{ checkingId === j.id ? '核查中…' : '查看详情' }}</button>
            </td>
          </tr>
          <tr v-if="!jobs.length">
            <td colspan="8" class="table-empty">{{ loading ? '正在读取任务…' : filter ? '没有“' + labels[filter] + '”任务，可切换其他状态。' : '暂无任务' }}</td>
          </tr>
        </tbody>
      </table>
    </div>
    <nav class="pagination" aria-label="任务分页">
      <span>第 {{ page }} / {{ pages }} 页 · 每页 {{ pageSize }} 条</span>
      <button class="btn small" :disabled="page <= 1 || loading" @click="page--">上一页</button>
      <button class="btn small" :disabled="page >= pages || loading" @click="page++">下一页</button>
    </nav>
    <Teleport to="body">
    <div v-if="selected && active" class="modal-backdrop" @click.self="closeDetail">
    <section class="modal task-dialog" role="dialog" aria-modal="true" aria-labelledby="task-detail-title" v-modal-focus="closeDetail">
      <header class="task-dialog-header">
        <h2 id="task-detail-title">任务 #{{ selected.id }} · {{ selected.title }}</h2>
        <button class="btn small" @click="closeDetail">关闭</button>
      </header>
      <div class="task-dialog-body">
      <div class="row task-actions">
        <span class="tag">{{ labels[selected.state] }}</span
        ><span>{{ selected.stage }}</span>
        <span v-if="selected.kind==='ai'" class="muted">{{ selected.request.provider==='qoder'?'Qoder CLI':'OpenCode' }} · {{ selected.request.model || '默认模型' }}</span>
        <div class="push-right row">
          <button class="btn small" :disabled="busy || checkingId !== null" @click="refreshDetail(selected.id)">{{ checkingId === selected.id ? '核查中…' : '重新核查状态' }}</button>
          <button
            v-if="['queued', 'running', 'waiting'].includes(selected.state)"
            class="btn small"
            :disabled="busy"
            @click="control(selected, 'pause')"
          >
            暂停</button
          ><button
            v-if="selected.state === 'paused'"
            class="btn small"
            :disabled="busy"
            @click="control(selected, 'resume')"
          >
            继续</button
          ><button
            v-if="['failed', 'partial'].includes(selected.state)"
            class="btn small"
            :disabled="busy"
            @click="control(selected, 'retry')"
          >
            重试失败项</button
          ><button
            v-if="
              ['queued', 'running', 'waiting', 'paused'].includes(
                selected.state,
              )
            "
            class="btn small"
            :disabled="busy"
            @click="control(selected, 'cancel')"
          >
            取消任务
          </button>
        </div>
      </div>
      <dl class="task-status" aria-label="后台任务状态">
        <div><dt>后台状态</dt><dd>{{ labels[selected.state] || selected.state }} <span class="muted">({{ selected.state }})</span></dd></div>
        <div><dt>处理进度</dt><dd>{{ selected.processed }} / {{ selected.total ?? '—' }}</dd></div>
        <div><dt>后台更新时间</dt><dd>{{ fullTime(selected.updatedAt) }}</dd></div>
        <div><dt>最近核查时间</dt><dd>{{ fullTime(checkedAt) }}</dd></div>
      </dl>
      <p v-if="detailError" class="error-panel" role="alert">{{ detailError }}</p>
      <p v-if="selected.error" class="error-panel">{{ selected.error }}</p>
      <div v-if="selected.result?.items" class="data-surface task-items">
      <table class="data-table">
        <tbody>
          <tr v-for="item in selected.result.items" :key="item.bookId">
            <td>
              <button
                class="text-button"
                @click="openResult('open-book', item.bookId)"
              >
                {{ item.title }}
              </button>
            </td>
            <td>{{ item.state === 'done' ? (selected.kind==='ai'?(selected.request.task==='sources'?'整理已保存':'草稿已保存'):'已完成') : item.state==='failed'?'失败':'待处理' }}<small v-if="item.error" class="danger">{{ item.error }}</small></td>
            <td class="row">
              <button v-if="selected.kind==='ai' && item.state==='done'" class="btn small" @click="openResult('open-draft',item.bookId,selected.request.task)">{{ selected.request.task==='sources'?'查看货源':'查看草稿' }}</button>
              <button
                v-if="item.state !== 'done' && selected.kind!=='ai'"
                class="btn small"
                :disabled="selected.state !== 'waiting'"
                @click="openResult('work-item', selected.kind, item)"
              >
                {{
                  { images: "找图", sources: "找货源", publish: "准备发布" }[
                    selected.kind
                  ]
                }}</button
              ><button
                v-if="item.state !== 'done' && selected.kind!=='ai'"
                class="btn small"
                :disabled="busy || selected.state !== 'waiting'"
                @click="
                  control(selected, 'complete-item', { bookId: item.bookId })
                "
              >
                核验完成
              </button>
            </td>
          </tr>
        </tbody>
      </table>
      </div>
      <TaskConversation v-if="selected.kind==='ai'" :key="selected.id" :job="selected" :active="active" />
      <template
        v-if="
          selected.kind === 'catalog' &&
          ['completed', 'partial'].includes(selected.state)
        "
        ><div class="result-line">
          <span>新增 {{ selected.result.import?.added || 0 }} 条用书</span
          ><span>变化 {{ selected.result.import?.changed || 0 }} 条</span
          ><button class="btn" @click="openResult('open-schools')">
            查看学校用书
          </button>
        </div>
        <ul v-if="selected.result.errors?.length" class="error-panel">
          <li v-for="(e, i) in selected.result.errors" :key="i">
            {{ typeof e === "string" ? e : JSON.stringify(e) }}
          </li>
        </ul></template
      >
      </div>
    </section>
    </div>
    </Teleport>
    <div v-if="creating" class="modal-backdrop" @click.self="creating = false">
      <section
        class="modal compact-dialog"
        v-modal-focus="
          () => {
            !busy && (creating = false);
          }
        "
        role="dialog"
        aria-modal="true"
        aria-labelledby="collect-title"
      >
        <header class="module-bar">
          <h2 id="collect-title">新建任务 · 学校用书采集</h2>
          <button class="btn small" @click="creating = false">关闭</button>
        </header>
        <form @submit.prevent="create">
          <label class="field"
            ><span>来源类型</span
            ><SearchSelect v-model="form.plugin" class="input"
              ><option v-for="a in adapters" :key="a.ref" :value="a.ref">
                {{ a.name }}
              </option></SearchSelect
            ></label
          ><label class="field"
            ><span>年份</span
            ><input
              v-model.number="form.year"
              type="number"
              min="2000"
              max="2200"
              class="input"
              required /></label
          ><label
            v-for="field in (adapter?.fields || []).filter(
              (f) => !['slug', 'title'].includes(f.name),
            )"
            :key="field.name"
            class="field"
            ><span>{{ field.label }}</span
            ><input
              v-model="form.args[field.name]"
              class="input"
              :required="field.required"
              :placeholder="field.placeholder || ''" /></label
          ><label class="check-line"
            ><input
              type="checkbox"
              v-model="form.refresh"
            />重新获取已缓存页面</label
          >
          <p v-if="error" role="alert" class="error-panel">{{ error }}</p>
          <button class="btn primary" :disabled="busy || !form.plugin">
            开始采集
          </button>
        </form>
      </section>
    </div>
  </section>
</template>

<style scoped>
.task-toolbar { flex-wrap: wrap; gap: 12px; }
.task-filter, .task-lookup { flex-wrap: wrap; }
.task-filter .input { width: auto; min-width: 168px; }
.task-lookup .input { width: 132px; }
.task-sync { justify-content: space-between; flex-wrap: wrap; margin-bottom: 12px; }
.task-table { min-width: 840px; }
.task-number { white-space: nowrap; font-variant-numeric: tabular-nums; }
.task-status { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 16px; margin: 16px 0; }
.task-status dt { color: var(--muted); margin-bottom: 4px; }
.task-status dd { margin: 0; overflow-wrap: anywhere; font-variant-numeric: tabular-nums; }
.modal.task-dialog { width: min(1040px, 100%); max-width: 100%; max-height: 90vh; padding: 0; display: flex; flex-direction: column; overflow: hidden; }
.task-dialog-header { display: flex; justify-content: space-between; align-items: flex-start; gap: 16px; padding: 20px 24px; border-bottom: 1px solid var(--line); }
.task-dialog-header h2 { margin: 0; overflow-wrap: anywhere; }
.task-dialog-header .btn { flex-shrink: 0; }
.task-dialog-body { padding: 20px 24px; overflow: auto; overscroll-behavior: contain; }
.task-actions { flex-wrap: wrap; }
.task-items { margin-top: 20px; }
@media (max-width: 800px) {
  .task-dialog-header, .task-dialog-body { padding: 16px; }
}
</style>
