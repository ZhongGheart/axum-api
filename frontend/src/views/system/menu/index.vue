<template>
  <div class="page-container">
    <n-page-header title="菜单管理">
      <template #extra>
        <PermissionButton :permission="PERM.MENU_CREATE" type="primary" @click="openCreate(null)">新增根菜单</PermissionButton>
      </template>
    </n-page-header>

    <n-card>
      <!--
        结构损坏的菜单被 build_tree 静默剪掉，在下面的树里根本看不到。
        这里把它们显出来，否则管理员只知道"少了菜单"，不知道被剪掉的是哪些、
        更不知道它们还留在库里。
      -->
      <n-alert
        v-if="unreachable.length > 0"
        type="error"
        :show-icon="true"
        class="broken-menu-alert"
      >
        <div class="broken-menu-title">
          有 {{ unreachable.length }} 个菜单不在菜单树里：它们无法从根节点到达，因此不会出现在侧栏和管理页中
        </div>
        <n-list>
          <n-list-item v-for="m in unreachable" :key="m.id">
            <div class="broken-menu-row">
              <span>{{ m.name }}</span>
              <n-tag size="small" type="error" :bordered="false">{{ m.reason }}</n-tag>
              <n-button size="tiny" type="warning" @click="handleDetach(m.id, m.name)">
                摘成根菜单
              </n-button>
            </div>
          </n-list-item>
        </n-list>
      </n-alert>

      <n-tree
        :data="treeData"
        :default-expand-all="true"
        :render-label="renderLabel"
        block-line
        checkable
        cascade
      />
    </n-card>

    <!-- 新增/编辑弹窗 -->
    <n-modal v-model:show="showModal" :title="isEditing ? '编辑菜单' : '新增菜单'" preset="card" style="width:560px">
      <n-form ref="formRef" :model="formData" :rules="rules" label-placement="left" label-width="80px">
        <n-form-item label="菜单名称" path="name">
          <n-input v-model:value="formData.name" />
        </n-form-item>
        <!--
          此前弹窗里根本没有上级字段，而提交时却会带上 `parentId` 的**上次残留值**：
          先在 A 节点点"新增子菜单"、再点 B 节点"编辑"保存，B 就被静默挂到 A 下，
          而界面上没有任何东西提示这件事。
        -->
        <n-form-item label="上级菜单" path="parent_id">
          <n-tree-select
            v-model:value="parentId"
            :options="parentOptions"
            :clearable="true"
            placeholder="不选则为顶级菜单"
            check-strategy="child"
          />
        </n-form-item>
        <n-form-item label="类型" path="type">
          <n-select v-model:value="formData.type" :options="typeOptions" />
        </n-form-item>
        <n-form-item label="路由路径" path="path">
          <n-input v-model:value="formData.path" placeholder="/system/user" />
        </n-form-item>
        <n-form-item label="图标" path="icon">
          <n-input v-model:value="formData.icon" placeholder="SettingsOutline" />
        </n-form-item>
        <n-form-item label="排序" path="sort_order">
          <n-input-number v-model:value="formData.sort_order" :min="0" />
        </n-form-item>
        <n-form-item label="权限标识" path="permission">
          <n-input v-model:value="formData.permission" placeholder="system:user:edit" />
        </n-form-item>
        <n-form-item label="是否可见">
          <n-switch v-model:value="formData.is_visible" />
        </n-form-item>
      </n-form>
      <template #footer>
        <n-space justify="end">
          <n-button @click="showModal = false">取消</n-button>
          <n-button type="primary" :loading="submitting" @click="handleSubmit">保存</n-button>
        </n-space>
      </template>
    </n-modal>
  </div>
</template>

<script setup lang="ts">
import { ref, h, onMounted } from 'vue'
import { NSpace, NIcon, NTooltip, NTag } from 'naive-ui'
import { AddOutline as AddIcon, CreateOutline as EditIcon, TrashOutline as DelIcon, RefreshOutline as RestoreIcon } from '@vicons/ionicons5'
import type { FormInst, FormRules, TreeOption } from 'naive-ui'
import { menuApi } from '@/api/menu'
import type { MenuNode, CreateMenuReq, UnreachableMenu } from '@/api/menu'
import { buildParentOptions } from '@/utils/menu'
import { PERM } from '@/constants/permission'
import { showConfirm, showSuccess } from '@/utils/message'
import PermissionButton from '@/components/common/PermissionButton.vue'

const formRef = ref<FormInst | null>(null)
const showModal = ref(false)
const isEditing = ref(false)
const editingId = ref('')
const parentId = ref<string | null>(null)
const submitting = ref(false)
const treeData = ref<TreeOption[]>([])
/**
 * 菜单 id → 可恢复的权限码
 *
 * 权限码被清空后全系统就没有任何角色再持有它，后端"改写权限码必须持有
 * 目标码"的守卫会把**写回**也一并拦死。这个映射让树行能显示恢复入口，
 * 否则管理员看到的就是一个"这个按钮怎么没权限了"的按钮，无从找回。
 */
const restorableById = ref<Record<string, string>>({})

/**
 * 走不到根、因而不在任何菜单树里的菜单
 *
 * 树本身看不见它们（后端静默剪掉了），所以只能单独查诊断接口。
 * 没有这一条时，成环的菜单会一直"人间蒸发"：管理员既找不到它，也就没法修。
 */
const unreachable = ref<UnreachableMenu[]>([])

/** 上级菜单候选项：编辑时排除自身与自身整棵子树 */
const parentOptions = ref<TreeOption[]>([])

/**
 * 后端返回的原始菜单树
 *
 * 单独留一份而不是从 `treeData`（`TreeOption[]`）反推：
 * 上级候选项要按 id 找父节点、按子树排除，两处都依赖 `MenuNode` 的原始形状。
 */
const rawTree = ref<MenuNode[]>([])

const typeOptions = [
  { label: '目录', value: 'directory' },
  { label: '菜单', value: 'menu' },
  { label: '按钮', value: 'button' },
]

interface MenuForm {
  name: string
  type: 'menu' | 'button' | 'directory'
  path: string
  icon: string
  sort_order: number
  permission: string
  is_visible: boolean
}

const formData = ref<MenuForm>({
  name: '', type: 'menu', path: '', icon: '', sort_order: 0, permission: '', is_visible: true,
})

const rules: FormRules = {
  name: [{ required: true, message: '请输入菜单名称' }],
  type: [{ required: true, message: '请选择类型' }],
}

async function fetchTree() {
  const res = await menuApi.list()
  const restorable: Record<string, string> = {}
  const collect = (nodes: MenuNode[]) => {
    for (const n of nodes) {
      if (n.restorable_permission) restorable[n.id] = n.restorable_permission
      if (n.children?.length) collect(n.children)
    }
  }
  rawTree.value = res as unknown as MenuNode[]
  collect(rawTree.value)
  restorableById.value = restorable
  treeData.value = buildTreeOptions(rawTree.value)
  // 响应拦截器已在运行时解包 `data`，但泛型签名仍标成 AxiosResponse<T>；
  // 本文件对 `list()` 用的是同一套处理方式。
  unreachable.value = (await menuApi.diagnostics()) as unknown as UnreachableMenu[]
}

function buildTreeOptions(nodes: MenuNode[]): TreeOption[] {
  return nodes.map((n) => ({
    key: n.id,
    label: n.name,
    children: n.children?.length ? buildTreeOptions(n.children) : undefined,
    isLeaf: !n.children?.length,
  }))
}

function renderLabel({ option }: { option: TreeOption }) {
  const restorable = restorableById.value[option.key as string]
  return h('div', { style: 'display:flex;align-items:center;gap:8px;padding:4px 0' }, [
    h('span', option.label as string),
    // 码被清空但留了恢复凭据：明确标出来，而不是让管理员对着一棵
    // "这个按钮怎么没权限了"的树猜
    restorable
      ? h(NTag, { size: 'small', type: 'warning', bordered: false }, {
          default: () => `码已清空，可恢复 ${restorable}`,
        })
      : null,
    h(NSpace, { size: 'small' }, {
      default: () => [
        // 行内三个入口按权限码显隐：v0.4.0 之前这里完全没接，
        // 等于权限码体系在菜单页自己身上漏了（后端仍会 403，但 UI 会误导）
        h(PermissionButton, { permission: PERM.MENU_CREATE, size: 'tiny', quaternary: true, onClick: () => openCreate(option.key as string) }, { default: () => h(NIcon, null, () => h(AddIcon)) }),
        h(PermissionButton, { permission: PERM.MENU_UPDATE, size: 'tiny', quaternary: true, onClick: () => openEdit(option.key as string) }, { default: () => h(NIcon, null, () => h(EditIcon)) }),
        restorable
          ? h(NTooltip, null, {
              trigger: () =>
                h(PermissionButton, { permission: PERM.MENU_UPDATE, size: 'tiny', quaternary: true, type: 'warning', onClick: () => handleRestore(option.key as string) }, { default: () => h(NIcon, null, () => h(RestoreIcon)) }),
              default: () => `恢复权限码 ${restorable}`,
            })
          : null,
        h(PermissionButton, { permission: PERM.MENU_DELETE, size: 'tiny', quaternary: true, type: 'error', onClick: () => handleDelete(option.key as string) }, { default: () => h(NIcon, null, () => h(DelIcon)) }),
      ],
    }),
  ])
}

async function openCreate(pid: string | null) {
  isEditing.value = false
  editingId.value = ''
  parentId.value = pid
  // 上级候选项此时不该排除任何节点：新建的菜单还不存在
  parentOptions.value = buildParentOptions(rawTree.value)
  formData.value = { name: '', type: 'menu', path: '', icon: '', sort_order: 0, permission: '', is_visible: true }
  showModal.value = true
}

async function openEdit(id: string) {
  isEditing.value = true
  editingId.value = id
  // 必须**重置** parentId：它是跨弹窗共享的 ref，
  // 留着上一次的"新增子菜单"目标就会在保存时静默把当前菜单挂过去。
  // 顺带把上级候选项算出来：排除自身与自身子树（挂到下级里会成环）。
  const flat = rawTree.value
  const findNode = (list: MenuNode[]): MenuNode | undefined => {
    for (const n of list) {
      if (n.id === id) return n
      const hit = findNode(n.children ?? [])
      if (hit) return hit
    }
    return undefined
  }
  const current = findNode(flat)
  parentId.value = current?.parent_id ?? null
  parentOptions.value = buildParentOptions(flat, id)
  showModal.value = true
}

async function handleSubmit() {
  try {
    await formRef.value?.validate()
    submitting.value = true
    // `parent_id` 必须显式发出去（含 `null`）：不传是"本次不改父级"，
    // 传 `null` 才是"摘成根"。用 `|| undefined` 会把两者混成前者。
    const data: CreateMenuReq = { ...formData.value, parent_id: parentId.value || null }
    if (isEditing.value) {
      await menuApi.update(editingId.value, data)
      showSuccess('更新成功')
    } else {
      await menuApi.create(data)
      showSuccess('创建成功')
    }
    showModal.value = false
    fetchTree()
  } catch { /* handled */ } finally { submitting.value = false }
}

/** 把不在树上的菜单摘成根，让它重新可见（诊断条上的修复入口） */
async function handleDetach(id: string, name: string) {
  const ok = await showConfirm({
    content: `把「${name}」摘成顶级菜单？它会立刻重新出现在菜单树里。`,
  })
  if (!ok) return
  try {
    await menuApi.update(id, { parent_id: null })
    showSuccess('已摘成顶级菜单')
    await fetchTree()
  } catch { /* handled */ }
}

async function handleDelete(id: string) {
  const ok = await showConfirm({ content: '确定删除该菜单及其子菜单？' })
  if (!ok) return
  await menuApi.delete(id)
  showSuccess('删除成功')
  fetchTree()
}

async function handleRestore(id: string) {
  const code = restorableById.value[id]
  const ok = await showConfirm({
    content: `确定恢复权限码「${code}」？只有清空者本人可以恢复。`,
  })
  if (!ok) return
  await menuApi.restorePermission(id)
  showSuccess('权限码已恢复')
  fetchTree()
}

onMounted(fetchTree)
</script>

<style scoped>
.broken-menu-alert {
  margin-bottom: 12px;
}

.broken-menu-title {
  font-weight: 500;
  margin-bottom: 8px;
}

.broken-menu-row {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
