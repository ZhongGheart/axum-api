<template>
  <div class="role-manage">
    <n-page-header title="角色管理" />

    <n-data-table
      :columns="columns"
      :data="roleList"
      :loading="loading"
      :bordered="true"
      class="role-table"
    />
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, h } from 'vue'
import { NTag } from 'naive-ui'
import type { DataTableColumn } from 'naive-ui'
import type { RoleItem } from '@/api/role'
import { roleApi } from '@/api/role'

const loading = ref(false)
const roleList = ref<RoleItem[]>([])

const columns: DataTableColumn[] = [
  { title: '角色名称', key: 'name', width: 120 },
  { title: '描述', key: 'description', width: 200 },
  {
    title: '用户数',
    key: 'user_count',
    width: 80,
    render(row: Record<string, unknown>) {
      return h(NTag, { type: 'info', size: 'small' }, () => String(row.user_count))
    },
  },
  { title: '创建时间', key: 'created_at', width: 180 },
]

async function fetchRoles() {
  loading.value = true
  try {
    const res = await roleApi.list()
    roleList.value = res as unknown as RoleItem[]
  } catch {
    // handled
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  fetchRoles()
})
</script>

<style scoped>
.role-manage {
  padding: 24px;
}
.role-table {
  margin-top: 16px;
}
</style>
