-- 同一字典类型下，`is_default` 最多只能有一个为真
--
-- "默认"这个词的全部意义就是唯一。v0.16.0 之前没有任何约束，
-- 实测可以连续创建任意多个默认项，界面上"默认"那一列出现多个开关同时打开，
-- 没有任何解释。仓储层已经在写路径上先取消旧默认项，但那是应用层保证：
-- 两个并发请求会各自看到"现在还没有默认项"，最后写两个。
-- 这个部分唯一索引是最后一道闸——并发下它会让后到的那个请求失败，
-- 而不是留下一个"有多个默认项"的数据集。
--
-- 建索引前先清理既有违规数据：否则迁移会在已有脏数据上直接失败，
-- 服务启动不了。每组保留 created_at 最早的一条（先设的先算默认）。
UPDATE dict_items AS d
SET is_default = FALSE
WHERE d.is_default
  AND d.id NOT IN (
      SELECT DISTINCT ON (dict_type_id) id
      FROM dict_items
      WHERE is_default
      ORDER BY dict_type_id, created_at ASC, id ASC
  );

CREATE UNIQUE INDEX IF NOT EXISTS idx_dict_items_single_default
    ON dict_items (dict_type_id)
    WHERE is_default;
