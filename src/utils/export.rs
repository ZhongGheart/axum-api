//! Excel 导出工具
//!
//! 基于 rust_xlsxwriter 实现数据导出，支持流式写入避免内存溢出。

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use rust_xlsxwriter::*;
use serde::Serialize;

use crate::error::AppError;

/// Excel 列定义
pub struct ExcelColumn {
    pub header: String,
    pub width: f64,
}

/// Excel 导出构建器
pub struct ExcelExport {
    workbook: Workbook,
    filename: String,
}

impl ExcelExport {
    /// 创建一个新的 Excel 导出实例
    pub fn new(filename: impl Into<String>) -> Self {
        Self {
            workbook: Workbook::new(),
            filename: filename.into(),
        }
    }

    /// 添加一个工作表并写入数据
    ///
    /// 使用 Serde 序列化自动适配字段名。
    /// 适用于结构体实现了 Serialize 的数据。
    pub fn add_sheet_from_serde<T: Serialize>(
        &mut self,
        sheet_name: &str,
        columns: &[ExcelColumn],
        data: &[T],
    ) -> Result<(), AppError> {
        let sheet = self.workbook.add_worksheet();
        // 设置工作表名
        sheet.set_name(sheet_name)?;

        // 写表头
        let header_format = Format::new()
            .set_bold()
            .set_background_color(Color::RGB(0xE8E8E8))
            .set_border(FormatBorder::Thin);

        for (col_idx, col) in columns.iter().enumerate() {
            sheet.write_string_with_format(0, col_idx as u16, &col.header, &header_format)?;
            sheet.set_column_width(col_idx as u16, col.width)?;
        }

        // 写数据行（通过 serde 序列化）
        for (row_idx, item) in data.iter().enumerate() {
            let row = row_idx as u32 + 1;
            if let Ok(value) = serde_json::to_value(item) {
                if let Some(obj) = value.as_object() {
                    for (col_idx, col) in columns.iter().enumerate() {
                        let val = obj.get(&col.header.to_lowercase())
                            .or_else(|| obj.get(&col.header))
                            .or_else(|| {
                                // 尝试驼峰匹配
                                let snake = col.header.to_lowercase().replace(' ', "_");
                                obj.get(&snake)
                            });

                        match val {
                            Some(v) if v.is_string() => {
                                sheet.write_string(row, col_idx as u16, v.as_str().unwrap_or(""))?;
                            }
                            Some(v) if v.is_number() => {
                                sheet.write_number(row, col_idx as u16, v.as_f64().unwrap_or(0.0))?;
                            }
                            Some(v) if v.is_boolean() => {
                                sheet.write_boolean(row, col_idx as u16, v.as_bool().unwrap_or(false))?;
                            }
                            _ => {
                                let text = val.map(|v| v.to_string()).unwrap_or_default();
                                // 去掉 JSON 引号
                                let clean = text.trim_matches('"');
                                sheet.write_string(row, col_idx as u16, clean)?;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// 添加一个工作表并写入行数据（手动构造）
    pub fn add_sheet_from_rows(
        &mut self,
        sheet_name: &str,
        columns: &[ExcelColumn],
        rows: &[Vec<String>],
    ) -> Result<(), AppError> {
        let sheet = self.workbook.add_worksheet();
        sheet.set_name(sheet_name)?;

        let header_format = Format::new()
            .set_bold()
            .set_background_color(Color::RGB(0xE8E8E8))
            .set_border(FormatBorder::Thin);

        for (col_idx, col) in columns.iter().enumerate() {
            sheet.write_string_with_format(0, col_idx as u16, &col.header, &header_format)?;
            sheet.set_column_width(col_idx as u16, col.width)?;
        }

        for (row_idx, row) in rows.iter().enumerate() {
            for (col_idx, cell) in row.iter().enumerate() {
                sheet.write_string(row_idx as u32 + 1, col_idx as u16, cell)?;
            }
        }

        Ok(())
    }

    /// 生成 Excel 响应（流式下载）
    pub fn into_response(mut self) -> Result<Response, AppError> {
        let data = self.workbook.save_to_buffer()
            .map_err(|e| AppError::InternalServerError(format!("Excel 生成失败: {e}")))?;

        let filename = urlencoding(&self.filename);

        let response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")
            .header(header::CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
            .header(header::CONTENT_LENGTH, data.len().to_string())
            .body(Body::from(data))
            .map_err(|e| AppError::InternalServerError(format!("响应构建失败: {e}")))?;

        Ok(response)
    }
}

/// URL 编码文件名（仅处理中文和空格）
fn urlencoding(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            ' ' => result.push_str("%20"),
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => result.push(c),
            _ => {
                for b in c.to_string().bytes() {
                    result.push_str(&format!("%{:02X}", b));
                }
            }
        }
    }
    result
}
