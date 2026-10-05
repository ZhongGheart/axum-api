//! Excel 导出工具
//!
//! 基于 rust_xlsxwriter 实现数据导出，支持流式写入避免内存溢出。

use axum::body::Body;
use axum::http::{header, StatusCode};
use axum::response::Response;
use rust_xlsxwriter::*;

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

    /// 添加一个工作表并写入行数据（手动构造）
    pub fn add_sheet_from_rows(
        &mut self,
        sheet_name: &str,
        columns: &[ExcelColumn],
        rows: &[Vec<String>],
    ) -> Result<(), AppError> {
        // 单元格与表头都是**按索引**写入的，两者长度不一致时不会报错，
        // 只会安静地产出一张参差的表：多出的单元格没有表头（取值读不出来），
        // 少掉的则是一列数据直接消失。
        //
        // 对审计导出这类**取证用途**的文件，静默少一列是最坏的结果：
        // 导出成功、行数正确、筛选也对，唯独缺了那唯一能回答问题的信息。
        // 因此在动手写之前先拒绝这种调用。
        for (idx, row) in rows.iter().enumerate() {
            if row.len() != columns.len() {
                return Err(AppError::InternalServerError(format!(
                    "导出工作表 {sheet_name:?} 第 {} 行的单元格数({})与表头数({})不一致",
                    idx + 1,
                    row.len(),
                    columns.len()
                )));
            }
        }

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
        let data = self
            .workbook
            .save_to_buffer()
            .map_err(|e| AppError::InternalServerError(format!("Excel 生成失败: {e}")))?;

        let filename = urlencoding(&self.filename);

        let response = Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            )
            .header(
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            )
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(n: usize) -> Vec<ExcelColumn> {
        (0..n)
            .map(|i| ExcelColumn {
                header: format!("列{i}"),
                width: 10.0,
            })
            .collect()
    }

    #[test]
    fn a_row_with_a_missing_cell_is_rejected_instead_of_silently_dropping_a_column() {
        // 这正是 v0.26.0 加「涉及对象」列时最容易踩的：
        // 表头加了、行忘了加，导出照样"成功"，只是那一列永远没有值
        let mut export = ExcelExport::new("t.xlsx");
        let err = export
            .add_sheet_from_rows("表", &cols(3), &[vec!["a".into(), "b".into()]])
            .expect_err("少一个单元格必须报错");
        assert!(
            err.to_string().contains("不一致"),
            "错误信息要点明是列数不一致: {err}"
        );
    }

    #[test]
    fn a_row_with_an_extra_cell_is_rejected_too() {
        let mut export = ExcelExport::new("t.xlsx");
        let err = export
            .add_sheet_from_rows(
                "表",
                &cols(2),
                &[vec!["a".into(), "b".into(), "多出来的".into()]],
            )
            .expect_err("多一个单元格必须报错");
        assert!(err.to_string().contains("不一致"), "{err}");
    }

    #[test]
    fn matching_lengths_still_write_the_sheet() {
        // 加上这道校验后，得确认真正常见的调用没被误伤
        let mut export = ExcelExport::new("t.xlsx");
        export
            .add_sheet_from_rows(
                "表",
                &cols(2),
                &[vec!["a".into(), "b".into()], vec!["c".into(), "d".into()]],
            )
            .expect("长度一致应当成功");
    }

    #[test]
    fn no_rows_at_all_is_fine() {
        // 筛不到数据是导出最常见的正常情形，不能因此报错
        let mut export = ExcelExport::new("t.xlsx");
        export
            .add_sheet_from_rows("表", &cols(2), &[])
            .expect("零行应当成功");
    }
}
