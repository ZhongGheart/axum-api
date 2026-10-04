/**
 * 部门管理页面
 *
 * 树形展示部门结构，支持新建、编辑、移动、删除。
 *
 * ── 为什么用 n-tree 而不是 n-data-table ──────────────────────
 * 部门是树形结构，n-tree 直接消费后端返回的树形数据，
 * 而 n-data-table 需要扁平数据 + 前端构建树。
 * 后端已经用 `build_tree` 构建好树，前端直接用。
 *
 * ── 为什么移动是独立操作 ────────────────────────────────────
 * 移动节点需要循环防护（不能把一个节点移动到自己的子孙下面），
 * 而修改名称不需要。把两者混在一个表单里会让校验逻辑变得复杂。
 */
<template>
  <div class="page-container">
    <n-page-header title="部门管理">
      <template #extra>
        <PermissionButton :permission="PERM.DEPT_CREATE" type="primary" @click="openCreate(null)">
          <template #icon><n-icon><AddIcon /></n-icon></template>
          新建部门
        </PermissionButton>
      </template>
    </n-page-header>

    <n-grid :cols="24" :x-gap="16">
      <!-- 部门树 -->
      <n-grid-item :span="10">
        <n-card title="部门结构" size="small">
          <n-spin :show="loading">
            <n-tree
              v-if="treeData.length > 0"
              :data="treeData"
              key-field="id"
              label-field="name"
              children-field="children"
              block-line
              selectable
              :selected-keys="selectedKeys"
              @update:selected-keys="onSelect"
            />
            <n-empty v-else description="暂无部门" size="small" />
          </n-spin>
        </n-card>
      </n-grid-item>

      <!-- 部门详情 / 编辑 -->
      <n-grid-item :span="14">
        <n-card v-if="selectedDept" :title="selectedDept.name" size="small">
          <template #header-extra>
            <n-space :size="8">
              <PermissionButton
                :permission="PERM.DEPT_CREATE"
                size="small"
                @click="openCreate(selectedDept.id)"
              >
                新建子部门
              </PermissionButton>
              <PermissionButton
                :permission="PERM.DEPT_UPDATE"
                size="small"
                @click="openEdit(selectedDept)"
              >
                编辑
              </PermissionButton>
              <PermissionButton
                :permission="PERM.DEPT_UPDATE"
                size="small"
                @click="openMove(selectedDept)"
              >
                移动
              </PermissionButton>
              <PermissionButton
                :permission="PERM.DEPT_DELETE"
                size="small"
                type="error"
                @click="handleDelete(selectedDept)"
              >
                删除
              </PermissionButton>
            </n-space>
          </template>

          <n-descriptions :column="1" label-placement="left" bordered size="small">
            <n-descriptions-item label="部门名称">
              {{ selectedDept.name }}
            </n-descriptions-item>
            <n-descriptions-item label="描述">
              {{ selectedDept.description || '未设置' }}
            </n-descriptions-item>
            <n-descriptions-item label="排序">
              {{ selectedDept.sort_order }}
            </n-descriptions-item>
            <n-descriptions-item label="子部门数">
              {{ selectedDept.children.length }}
            </n-descriptions-item>
          </n-descriptions>

          <n-divider />

          <div class="hint">
            点击左侧树节点查看部门详情。选中父部门后点「新建子部门」可快速创建。
          </div>
        </n-card>

        <n-card v-else title="部门详情" size="small">
          <n-empty description="请在左侧选择部门" size="small" />
        </n-card>
      </n-grid-item>
    </n-grid>

    <!-- 新建/编辑部门 -->
    <n-modal
      v-model:show="showModal"
      :title="isEditing ? '编辑部门' : '新建部门'"
      :mask-closable="false"
      preset="card"
      style="width: 480px"
    >
      <n-form ref="formRef" :model="formData" :rules="formRules" label-placement="left" label-width="80px">
        <n-form-item label="部门名称" path="name">
          <n-input v-model:value="formData.name" :maxlength="100" placeholder="如：技术部" />
        </n-form-item>
        <n-form-item label="描述" path="description">
          <n-input v-model:value="formData.description" type="textarea" :maxlength="200" :rows="2" />
        </n-form-item>
        <n-form-item label="排序" path="sortOrder">
          <n-input-number v-model:value="formData.sortOrder" :min="0" :max="9999" style="width: 100%" />
        </n-form-item>
      </n-form>
      <template #footer>
        <n-space justify="end">
          <n-button @click="showModal = false">取消</n-button>
          <n-button type="primary" :loading="submitting" @click="handleSubmit">保存</n-button>
        </n-space>
      </template>
    </n-modal>

    <!-- 移动部门 -->
    <n-modal
      v-model:show="showMoveModal"
      title="移动部门"
      :mask-closable="false"
      preset="card"
      style="width: 480px"
    >
      <n-form label-placement="left" label-width="80px">
        <n-form-item label="移动到">
          <n-tree-select
            v-model:value="moveTargetId"
            :data="flatOptions"
            key-field="id"
            label-field="name"
            children-field="children"
            placeholder="选择父部门（不选则移到根）"
            clearable
          />
        </n-form-item>
      </n-form>
      <template #footer>
        <n-space justify="end">
          <n-button @click="showMoveModal = false">取消</n-button>
          <n-button type="primary" :loading="moving" @click="handleMove">移动</n-button>
        </n-space>
      </template>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import type { FormInst, FormRules, TreeOption } from 'naive-ui'
import { AddOutline as AddIcon } from '@vicons/ionicons5'
import PermissionButton from '@/components/common/PermissionButton.vue'
import { PERM } from '@/constants/permission'
import { departmentApi } from '@/api/department'
import type { DepartmentFlat, DepartmentNode } from '@/api/types/response'
import { showSuccess } from '@/utils/message'

// ── 部门树 ────────────────────────────────────────────────────

const treeData = ref<DepartmentNode[]>([])
const loading = ref(false)
const selectedKeys = ref<string[]>([])

const selectedDept = computed<DepartmentNode | null>(() => {
  if (selectedKeys.value.length === 0) return null
  return findNode(treeData.value, selectedKeys.value[0])
})

function findNode(nodes: DepartmentNode[], id: string): DepartmentNode | null {
  for (const node of nodes) {
    if (node.id === id) return node
    const found = findNode(node.children, id)
    if (found) return found
  }
  return null
}

function onSelect(keys: string[]) {
  selectedKeys.value = keys
}

async function fetchTree() {
  loading.value = true
  try {
    treeData.value = await departmentApi.tree()
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    loading.value = false
  }
}

// ── 新建 / 编辑 ───────────────────────────────────────────────

const showModal = ref(false)
const isEditing = ref(false)
const editingId = ref<string | null>(null)
const submitting = ref(false)
const formRef = ref<FormInst | null>(null)

const formData = reactive({
  name: '',
  description: '',
  sortOrder: 0,
})

const formRules: FormRules = {
  name: [
    { required: true, message: '请输入部门名称', trigger: ['input', 'blur'] },
    {
      trigger: ['input', 'blur'],
      validator: (_rule, value: string) =>
        (value ?? '').trim().length <= 100
          ? true
          : new Error('部门名称不能超过 100 个字符'),
    },
  ],
}

function openCreate(parentId: string | null) {
  isEditing.value = false
  editingId.value = parentId
  formData.name = ''
  formData.description = ''
  formData.sortOrder = 0
  showModal.value = true
}

function openEdit(dept: DepartmentNode) {
  isEditing.value = true
  editingId.value = dept.id
  formData.name = dept.name
  formData.description = dept.description ?? ''
  formData.sortOrder = dept.sort_order
  showModal.value = true
}

async function handleSubmit() {
  if (submitting.value) return
  try {
    await formRef.value?.validate()
  } catch {
    return
  }
  submitting.value = true
  try {
    if (isEditing.value && selectedDept.value) {
      await departmentApi.update(selectedDept.value.id, {
        name: formData.name.trim(),
        description: formData.description.trim() || undefined,
        sort_order: formData.sortOrder,
      })
      showSuccess('部门已更新')
    } else {
      await departmentApi.create({
        parent_id: editingId.value,
        name: formData.name.trim(),
        description: formData.description.trim() || undefined,
        sort_order: formData.sortOrder,
      })
      showSuccess('部门已创建')
    }
    showModal.value = false
    await fetchTree()
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    submitting.value = false
  }
}

// ── 移动部门 ──────────────────────────────────────────────────

const showMoveModal = ref(false)
const moving = ref(false)
const moveTargetId = ref<string | null>(null)
const flatOptions = ref<TreeOption[]>([])

async function openMove(dept: DepartmentNode) {
  moveTargetId.value = dept.parent_id
  showMoveModal.value = true
  // 加载扁平列表用于选择父部门
  try {
    const list = await departmentApi.flatList()
    flatOptions.value = list.map((d: DepartmentFlat) => ({
      id: d.id,
      name: d.path,
      children: [],
    }))
  } catch {
    // 错误提示已由响应拦截器统一弹出
  }
}

async function handleMove() {
  if (moving.value || !selectedDept.value) return
  moving.value = true
  try {
    await departmentApi.move(selectedDept.value.id, {
      new_parent_id: moveTargetId.value,
    })
    showSuccess('部门已移动')
    showMoveModal.value = false
    await fetchTree()
  } catch {
    // 错误提示已由响应拦截器统一弹出
  } finally {
    moving.value = false
  }
}

// ── 删除部门 ──────────────────────────────────────────────────

async function handleDelete(dept: DepartmentNode) {
  try {
    await departmentApi.remove(dept.id)
    showSuccess('部门已删除')
    selectedKeys.value = []
    await fetchTree()
  } catch {
    // 错误提示已由响应拦截器统一弹出
  }
}

onMounted(() => {
  void fetchTree()
})
</script>

<style scoped>
.page-container {
  padding: 16px;
}

.hint {
  font-size: 12px;
  color: var(--n-text-color-3, #999);
  margin-top: 8px;
}
</style>
