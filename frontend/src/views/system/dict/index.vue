<template>
  <div class="dict-page">
    <n-page-header title="字典管理">
      <template #extra>
        <n-button quaternary @click="handleRefreshCache">刷新缓存</n-button>
        <n-button type="primary" @click="openCreateType">新增字典</n-button>
      </template>
    </n-page-header>

    <n-split direction="horizontal" :default-size="0.35">
      <!-- 左侧：字典类型列表 -->
      <template #left>
        <n-card title="字典类型" size="small" :bordered="false">
          <template #header-extra>
            <n-button size="tiny" quaternary @click="openCreateType">
              <template #icon><n-icon><AddIcon /></n-icon></template>
            </n-button>
          </template>
          <n-list hoverable clickable>
            <n-list-item
              v-for="t in types"
              :key="t.id"
              :class="{ active: selectedType?.id === t.id }"
              @click="selectType(t)"
            >
              <n-thing :title="t.name" :description="`${t.code} (${t.status})`" />
            </n-list-item>
          </n-list>
        </n-card>
      </template>

      <!-- 右侧：字典项列表 -->
      <template #right>
        <n-card v-if="selectedType" :title="`${selectedType.name} - 字典项`" size="small" :bordered="false">
          <template #header-extra>
            <n-button size="tiny" quaternary @click="openEditType(selectedType)">编辑类型</n-button>
            <n-button size="tiny" quaternary @click="handleDeleteType(selectedType.id)">删除类型</n-button>
            <n-button size="tiny" type="primary" @click="openCreateItem">新增项</n-button>
          </template>

          <n-data-table
            :columns="itemColumns"
            :data="items"
            :loading="loadingItems"
            :bordered="false"
            size="small"
          />
        </n-card>
        <n-empty v-else description="请从左列表选择一个字典类型" style="margin-top:80px" />
      </template>
    </n-split>

    <!-- 字典类型 Dialog -->
    <n-modal v-model:show="showTypeDialog" :title="isEditingType ? '编辑字典类型' : '新增字典类型'" preset="card" style="width:480px">
      <n-form ref="typeFormRef" :model="typeForm" :rules="typeRules" label-placement="left" label-width="80px">
        <n-form-item label="编码" path="code"><n-input v-model:value="typeForm.code" /></n-form-item>
        <n-form-item label="名称" path="name"><n-input v-model:value="typeForm.name" /></n-form-item>
        <n-form-item label="描述" path="description"><n-input v-model:value="typeForm.description" type="textarea" /></n-form-item>
        <n-form-item label="状态" path="status">
          <n-select v-model:value="typeForm.status" :options="[{label:'启用',value:'enabled'},{label:'禁用',value:'disabled'}]" />
        </n-form-item>
        <n-form-item label="排序" path="sort_order"><n-input-number v-model:value="typeForm.sort_order" :min="0" /></n-form-item>
      </n-form>
      <template #footer>
        <n-space justify="end">
          <n-button @click="showTypeDialog = false">取消</n-button>
          <n-button type="primary" :loading="submittingType" @click="handleSubmitType">保存</n-button>
        </n-space>
      </template>
    </n-modal>

    <!-- 字典项 Dialog -->
    <n-modal v-model:show="showItemDialog" :title="isEditingItem ? '编辑字典项' : '新增字典项'" preset="card" style="width:480px">
      <n-form ref="itemFormRef" :model="itemForm" :rules="itemRules" label-placement="left" label-width="80px">
        <n-form-item label="标签" path="label"><n-input v-model:value="itemForm.label" /></n-form-item>
        <n-form-item label="值" path="value"><n-input v-model:value="itemForm.value" /></n-form-item>
        <n-form-item label="排序" path="sort_order"><n-input-number v-model:value="itemForm.sort_order" :min="0" /></n-form-item>
        <n-form-item label="状态" path="status">
          <n-select v-model:value="itemForm.status" :options="[{label:'启用',value:'enabled'},{label:'禁用',value:'disabled'}]" />
        </n-form-item>
        <n-form-item label="默认">
          <n-switch v-model:value="itemForm.is_default" />
        </n-form-item>
        <n-form-item label="颜色" path="color"><n-input v-model:value="itemForm.color" placeholder="#1890ff" /></n-form-item>
      </n-form>
      <template #footer>
        <n-space justify="end">
          <n-button @click="showItemDialog = false">取消</n-button>
          <n-button type="primary" :loading="submittingItem" @click="handleSubmitItem">保存</n-button>
        </n-space>
      </template>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, onMounted, h } from 'vue'
import { NTag, NButton, NSwitch, useMessage } from 'naive-ui'
import { AddOutline as AddIcon } from '@vicons/ionicons5'
import type { DataTableColumn, FormInst, FormRules } from 'naive-ui'
import { dictApi } from '@/api/dict'
import type { DictTypeItem, DictItemRecord } from '@/api/dict'
import { showSuccess, showConfirm, showError } from '@/utils/message'

const message = useMessage()

// ── Types ──────────────────────────────────────────────
const types = ref<DictTypeItem[]>([])
const selectedType = ref<DictTypeItem | null>(null)
const items = ref<DictItemRecord[]>([])
const loadingItems = ref(false)

// ── Type Dialog ────────────────────────────────────────
const showTypeDialog = ref(false)
const isEditingType = ref(false)
const editingTypeId = ref('')
const submittingType = ref(false)
const typeFormRef = ref<FormInst | null>(null)
const typeForm = reactive({
  code: '', name: '', description: '', status: 'enabled', sort_order: 0,
})
const typeRules: FormRules = {
  code: [{ required: true, message: '请输入编码' }],
  name: [{ required: true, message: '请输入名称' }],
}

// ── Item Dialog ────────────────────────────────────────
const showItemDialog = ref(false)
const isEditingItem = ref(false)
const editingItemId = ref('')
const submittingItem = ref(false)
const itemFormRef = ref<FormInst | null>(null)
const itemForm = reactive({
  label: '', value: '', sort_order: 0, status: 'enabled', is_default: false, color: '',
})
const itemRules: FormRules = {
  label: [{ required: true, message: '请输入标签' }],
  value: [{ required: true, message: '请输入值' }],
}

// ── Columns ────────────────────────────────────────────
const itemColumns: DataTableColumn[] = [
  { title: '标签', key: 'label', width: 120 },
  { title: '值', key: 'value', width: 120 },
  { title: '排序', key: 'sort_order', width: 60 },
  {
    title: '颜色', key: 'color', width: 80,
    render(row: Record<string, unknown>) {
      const r = row as unknown as DictItemRecord
      return r.color ? h('span', { style: `color:${r.color}` }, '■') : '-'
    },
  },
  {
    title: '状态', key: 'status', width: 70,
    render(row: Record<string, unknown>) {
      const r = row as unknown as DictItemRecord
      return h(NTag, { type: r.status === 'enabled' ? 'success' : 'default', size: 'tiny' },
        () => r.status === 'enabled' ? '启用' : '禁用')
    },
  },
  {
    title: '默认', key: 'is_default', width: 60,
    render(row: Record<string, unknown>) {
      const r = row as unknown as DictItemRecord
      return h(NSwitch, { value: r.is_default, disabled: true, size: 'small' })
    },
  },
  {
    title: '操作', key: 'actions', width: 100,
    render(row: Record<string, unknown>) {
      const r = row as unknown as DictItemRecord
      return h('div', { style: 'display:flex;gap:4px' }, [
        h(NButton, { size: 'tiny', quaternary: true, onClick: () => openEditItem(r) }, () => '编辑'),
        h(NButton, { size: 'tiny', quaternary: true, type: 'error', onClick: () => handleDeleteItem(r.id) }, () => '删除'),
      ])
    },
  },
]

// ── Actions ────────────────────────────────────────────
async function fetchTypes() {
  try { types.value = (await dictApi.listTypes()) as unknown as DictTypeItem[] }
  catch { message.error('加载字典类型失败') }
}

async function selectType(t: DictTypeItem) {
  selectedType.value = t
  loadingItems.value = true
  try { items.value = (await dictApi.listItems(t.id)) as unknown as DictItemRecord[] }
  catch { items.value = [] }
  finally { loadingItems.value = false }
}

// ── Type CRUD ──────────────────────────────────────────
function openCreateType() {
  isEditingType.value = false; editingTypeId.value = ''
  Object.assign(typeForm, { code: '', name: '', description: '', status: 'enabled', sort_order: 0 })
  showTypeDialog.value = true
}

function openEditType(t: DictTypeItem) {
  isEditingType.value = true; editingTypeId.value = t.id
  Object.assign(typeForm, { code: t.code, name: t.name, description: t.description || '', status: t.status, sort_order: t.sort_order })
  showTypeDialog.value = true
}

async function handleSubmitType() {
  try {
    await typeFormRef.value?.validate()
    submittingType.value = true
    if (isEditingType.value) {
      await dictApi.updateType(editingTypeId.value, { ...typeForm })
      showSuccess('更新成功')
    } else {
      await dictApi.createType({ ...typeForm })
      showSuccess('创建成功')
    }
    showTypeDialog.value = false
    await fetchTypes()
  } catch { /* */ } finally { submittingType.value = false }
}

async function handleDeleteType(id: string) {
  const ok = await showConfirm({ content: '删除字典类型同时会删除其下所有字典项，确认？' })
  if (!ok) return
  await dictApi.deleteType(id)
  showSuccess('删除成功')
  selectedType.value = null
  items.value = []
  await fetchTypes()
}

// ── Item CRUD ──────────────────────────────────────────
function openCreateItem() {
  isEditingItem.value = false; editingItemId.value = ''
  Object.assign(itemForm, { label: '', value: '', sort_order: 0, status: 'enabled', is_default: false, color: '' })
  showItemDialog.value = true
}

function openEditItem(it: DictItemRecord) {
  isEditingItem.value = true; editingItemId.value = it.id
  Object.assign(itemForm, { label: it.label, value: it.value, sort_order: it.sort_order, status: it.status, is_default: it.is_default, color: it.color || '' })
  showItemDialog.value = true
}

async function handleSubmitItem() {
  if (!selectedType.value) return
  try {
    await itemFormRef.value?.validate()
    submittingItem.value = true
    const payload = { ...itemForm, dict_type_id: isEditingItem.value ? undefined : selectedType.value.id }
    if (isEditingItem.value) {
      await dictApi.updateItem(editingItemId.value, payload)
      showSuccess('更新成功')
    } else {
      await dictApi.createItem(payload)
      showSuccess('创建成功')
    }
    showItemDialog.value = false
    if (selectedType.value) await selectType(selectedType.value)
  } catch { /* */ } finally { submittingItem.value = false }
}

async function handleDeleteItem(id: string) {
  const ok = await showConfirm({ content: '确定删除该字典项？' })
  if (!ok) return
  await dictApi.deleteItem(id)
  showSuccess('删除成功')
  if (selectedType.value) await selectType(selectedType.value)
}

async function handleRefreshCache() {
  try {
    await dictApi.refreshCache()
    showSuccess('缓存刷新成功')
  } catch { showError('缓存刷新失败') }
}

onMounted(fetchTypes)
</script>

<style scoped>
.dict-page { height: calc(100vh - 100px); }
.dict-page :deep(.n-split) { height: 100%; }
.active { background-color: var(--primary-color-hover, #e6f7ff); }
</style>
