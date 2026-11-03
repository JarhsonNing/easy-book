<script setup>
import { onMounted, reactive, ref } from "vue";
import { api } from "../../api";
import AgentSettings from './AgentSettings.vue';
import AiSettings from './AiSettings.vue';
import RegionSelect from '../RegionSelect.vue';
import { isValidRegion } from '../../regions';
import { listOf } from '../../xianyu';
const props = defineProps({ active: { type: Boolean, default: true } });
const emit = defineEmits(["updated", "open-opencode", "open-qoder"]);
const tab = ref("platform"),
  busy = ref(false),
  error = ref(""),
  message = ref(""),
  configured = ref(false),
  hostingConfigured = ref(false);
const agentsVisited=ref(false);
const agentProvider=ref('opencode');
const aiVisited=ref(false);
function showAgents(provider='opencode') { agentProvider.value=provider; tab.value = 'agents'; agentsVisited.value = true; }
function showAi(){tab.value='ai';aiVisited.value=true;}
defineExpose({ showAgents, showAi });
const credentials = reactive({ appKey: "", appSecret: "" }),
  hosting = reactive({
    repoUrl: "",
    token: "",
    branch: "",
    directory: "taoshu-images",
  }),
  settings = reactive({});
const publishDefaults = reactive({ channelCatId: '', channelCatName: '教材教辅', province: 330000, city: 330100, district: 330101, stock: 9999, stuffStatus: 90 });
const categories = ref([]), categoriesLoading = ref(false), categoriesError = ref('');
const whitelistText = ref('');
async function saveWhitelist() {
  await action(async () => {
    const saved = await api.saveSettings({ sourceShopWhitelist: whitelistText.value.split(/\r?\n/).map(s => s.trim()).filter(Boolean) });
    Object.assign(settings, saved);
  });
}
async function loadCategories() {
  categoriesLoading.value = true;
  categoriesError.value = '';
  try {
    categories.value = listOf(await api.xianyuCall('categories', { item_biz_type: 2, sp_biz_type: 24 }));
  } catch (e) {
    categoriesError.value = e.message;
  } finally {
    categoriesLoading.value = false;
  }
}
function savePublishDefaults() {
  return action(async () => {
    const hasRegion = publishDefaults.province || publishDefaults.city || publishDefaults.district;
    if (hasRegion && !isValidRegion(publishDefaults)) throw new Error('请选择完整的默认省、市、区');
    const saved = await api.saveSettings({ xianyuPublishDefaults: {
      channelCatId: String(publishDefaults.channelCatId),
      province: Number(publishDefaults.province),
      city: Number(publishDefaults.city),
      district: Number(publishDefaults.district),
      channelCatName: categories.value.find(c => String(c.channel_cat_id) === String(publishDefaults.channelCatId))?.channel_cat_name || publishDefaults.channelCatName,
      stock: Number(publishDefaults.stock), stuffStatus: Number(publishDefaults.stuffStatus),
    } });
    Object.assign(settings, saved);
  });
}
async function action(fn) {
  busy.value = true;
  error.value = "";
  message.value = "";
  try {
    await fn();
    message.value = "已保存";
    emit("updated");
  } catch (e) {
    error.value = e.message;
  } finally {
    busy.value = false;
  }
}
async function load() {
  try {
    const [c, h, s] = await Promise.all([
      api.xianyuConfig(),
      api.giteeConfig(),
      api.settings(),
    ]);
    credentials.appKey = c.appKey || "";
    configured.value = c.secretConfigured;
    Object.assign(hosting, {
      repoUrl: h.repoUrl || "",
      branch: h.branch || "",
      directory: h.directory || "taoshu-images",
    });
    hostingConfigured.value = h.tokenConfigured;
    Object.assign(settings, s);
    whitelistText.value = (s.sourceShopWhitelist || []).join('\n');
    if (s.xianyuPublishDefaults) Object.assign(publishDefaults, {
      channelCatId: s.xianyuPublishDefaults.channelCatId || '',
      province: s.xianyuPublishDefaults.province || '',
      city: s.xianyuPublishDefaults.city || '',
      district: s.xianyuPublishDefaults.district || '',
      channelCatName: s.xianyuPublishDefaults.channelCatName || '教材教辅',
      stock: s.xianyuPublishDefaults.stock || 9999, stuffStatus: s.xianyuPublishDefaults.stuffStatus ?? 90,
    });
  } catch (e) {
    error.value = e.message;
  }
}
async function saveConnection() {
  await action(async () => {
    await api.saveXianyuConfig({ ...credentials });
    credentials.appSecret = "";
    await load();
  });
}
async function test() {
  await action(async () => {
    await api.xianyuCall("shops");
  });
  if (!error.value) message.value = "连接成功";
}
async function saveHosting() {
  await action(async () => {
    await api.saveGiteeConfig({ ...hosting });
    hosting.token = "";
    await load();
  });
}
async function testHosting() {
  await action(() => api.testGitee());
  if (!error.value) message.value = "仓库连接正常";
}
onMounted(load);
</script>
<template>
  <section class="settings-page">
    <nav class="page-tabs" aria-label="设置分类">
      <button
        v-for="(label, key) in {
          platform: '平台连接',
          images: '图片托管',
          pricing: '定价与文案',
          publishing: '发布默认值',
          sources: '货源白名单',
          app: '应用',
          agents: 'Agent 接入',
          ai: 'AI 流程',
        }"
        :key="key"
        :class="{ active: tab === key }"
        @click="tab = key; if(key==='agents')agentsVisited=true; if(key==='ai')aiVisited=true; if(key==='publishing')loadCategories()"
      >
        {{ label }}
      </button>
    </nav>
    <p v-if="error" class="error-panel" role="alert">{{ error }}</p>
    <p v-if="message" class="success-message" role="status">{{ message }}</p>
    <AgentSettings v-if="agentsVisited" v-show="tab==='agents'" :key="agentProvider" :active="props.active && tab==='agents'" :selected-provider="agentProvider" @open-opencode="emit('open-opencode')" @open-qoder="emit('open-qoder')" />
    <AiSettings v-if="aiVisited" v-show="tab==='ai'" @agent-settings="showAgents" />
    <form
      v-if="tab === 'platform'"
      class="settings-form"
      @submit.prevent="saveConnection"
    >
      <div class="module-bar">
        <h2>闲管家</h2>
        <span class="tag" :class="configured ? 'success' : 'neutral'">{{
          configured ? "已配置" : "未配置"
        }}</span>
      </div>
      <label class="field"
        ><span>AppKey</span
        ><input
          v-model="credentials.appKey"
          class="input"
          required
          autocomplete="off" /></label
      ><label class="field"
        ><span>AppSecret</span
        ><input
          v-model="credentials.appSecret"
          type="password"
          class="input"
          :placeholder="configured ? '已保存，留空保留' : ''"
          autocomplete="new-password"
      /></label>
      <div class="row">
        <button class="btn primary" :disabled="busy">保存</button
        ><button
          class="btn"
          type="button"
          :disabled="busy || !configured"
          @click="test"
        >
          测试连接
        </button>
      </div>
      <details class="detail-section">
        <summary>店铺授权</summary>
        <p>在闲管家完成店铺授权后，在订单页切换店铺。</p>
      </details>
    </form>
    <form
      v-if="tab === 'publishing'"
      class="settings-form"
      @submit.prevent="savePublishDefaults"
    >
      <h2>发布默认值</h2>
      <p class="hint">打开发布窗口时自动带入，可在单次发布中修改。</p>
      <fieldset class="xy-fieldset publish-defaults" :disabled="busy">
        <label class="field">
          <span>图书类目</span>
          <SearchSelect v-model="publishDefaults.channelCatId" class="input" :disabled="categoriesLoading">
            <option value="">不设置默认类目</option>
            <option v-if="publishDefaults.channelCatId && !categories.some(cat => String(cat.channel_cat_id) === publishDefaults.channelCatId)" :value="publishDefaults.channelCatId">已保存类目（{{ publishDefaults.channelCatId }}）</option>
            <option v-for="cat in categories" :key="cat.channel_cat_id" :value="String(cat.channel_cat_id)">{{ cat.channel_cat_name }}</option>
          </SearchSelect>
        </label>
        <p v-if="categoriesLoading" class="hint" role="status">正在加载类目…</p>
        <div v-if="categoriesError" class="row">
          <p class="error-panel" role="alert">{{ categoriesError }}</p>
          <button class="btn" type="button" @click="loadCategories">重新加载</button>
        </div>
        <RegionSelect :model-value="publishDefaults" :required="false" @update:model-value="Object.assign(publishDefaults, $event)" />
        <label class="field"><span>默认库存</span><input v-model.number="publishDefaults.stock" type="number" min="1" max="9999" class="input" required /></label>
        <p class="hint">默认成色为九成新；类目提供“轻微使用痕迹”属性时自动选中。</p>
        <div class="row">
          <button class="btn primary" :disabled="busy">{{ busy ? '保存中…' : '保存默认值' }}</button>
          <button class="btn" type="button" @click="Object.assign(publishDefaults, { channelCatId: '', province: '', city: '', district: '' })">清空</button>
        </div>
      </fieldset>
    </form>
    <form v-if="tab === 'sources'" class="settings-form" @submit.prevent="saveWhitelist">
      <h2>货源店铺白名单</h2>
      <p class="hint">每行一个完整店名。匹配店铺且有采购价和 ISBN 时，自动标为可用并启用代发；品相默认九成新、运费默认 0。</p>
      <label class="field"><span>免核验店铺</span><textarea v-model="whitelistText" class="input whitelist-input" rows="8" /></label>
      <button class="btn primary" :disabled="busy">保存白名单</button>
    </form>
    <form
      v-if="tab === 'images'"
      class="settings-form"
      @submit.prevent="saveHosting"
    >
      <div class="module-bar">
        <h2>Gitee 图片托管</h2>
        <span class="tag">{{ hostingConfigured ? "已配置" : "未配置" }}</span>
      </div>
      <label
        v-for="(label, key) in {
          repoUrl: '仓库地址',
          token: '访问令牌',
          branch: '分支',
          directory: '图片目录',
        }"
        :key="key"
        class="field"
        ><span>{{ label }}</span
        ><input
          v-model="hosting[key]"
          class="input"
          :type="key === 'token' ? 'password' : 'text'"
          :placeholder="
            key === 'token' && hostingConfigured ? '已保存，留空保留' : ''
          "
      /></label>
      <div class="row">
        <button class="btn primary" :disabled="busy">保存</button
        ><button
          type="button"
          class="btn"
          :disabled="busy || !hostingConfigured"
          @click="testHosting"
        >
          测试连接
        </button>
      </div>
      <details class="detail-section">
        <summary>图片访问要求</summary>
        <p>发布图片需要可匿名读取的公开仓库。上传图片公开可见。</p>
      </details>
    </form>
    <form
      v-if="tab === 'pricing'"
      class="settings-form"
      @submit.prevent="action(() => api.saveSettings(settings))"
    >
      <h2>定价与文案</h2>
      <div class="form-grid">
        <label class="field"
          ><span>成本加价率（%）</span
          ><input
            v-model.number="settings.profitRate"
            class="input"
            type="number"
            min="10"
            max="20" /></label
        ><label class="field"
          ><span>最低加价（元）</span
          ><input
            v-model.number="settings.minProfit"
            class="input"
            type="number"
            min="0"
            step="0.01" /></label
        ><label class="field"
          ><span>默认品相</span
          ><input v-model="settings.defaultCondition" class="input" /></label
        ><label class="field"
          ><span>标题后缀</span
          ><input v-model="settings.titleSuffix" class="input" /></label
        ><label class="field wide"
          ><span>描述附加内容</span
          ><textarea v-model="settings.descFooter" class="input" rows="4" />
        </label>
      </div>
      <button class="btn primary" :disabled="busy">保存</button>
    </form>
    <form
      v-if="tab === 'app'"
      class="settings-form"
      @submit.prevent="action(() => api.saveSettings(settings))"
    >
      <h2>应用设置</h2>
      <label class="field"
        ><span>孔夫子搜索地址</span
        ><input v-model="settings.kongfzUrl" class="input" type="url" /></label
      ><label class="field"
        ><span>闲鱼网页发布地址</span
        ><input v-model="settings.publishUrl" class="input" type="url" /></label
      ><label class="check-line"
        ><input
          type="checkbox"
          v-model="settings.autoSubmit"
        />网页发布自动提交</label
      ><button class="btn primary" :disabled="busy">保存</button>
    </form>
  </section>
</template>

<style scoped>
.publish-defaults { display: grid; gap: 16px; }
.publish-defaults .row { margin-top: 0; }
.whitelist-input { min-height: 180px; }
</style>
