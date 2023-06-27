use crate::capabilities::{attempt_server_capability, CAPABILITY_FORMATTING};
use crate::context::*;
use crate::types::*;
use lsp_types::request::*;
use lsp_types::*;
use serde::Deserialize;
use url::Url;

pub fn text_document_formatting(meta: EditorMeta, params: EditorParams, ctx: &mut Context) {
    let entry = ctx.language_servers.first_key_value().unwrap();
    if meta.fifo.is_none() && !attempt_server_capability(entry, &meta, CAPABILITY_FORMATTING) {
        return;
    }

    let params = FormattingOptions::deserialize(params)
        .expect("Params should follow FormattingOptions structure");
    let req_params = DocumentFormattingParams {
        text_document: TextDocumentIdentifier {
            uri: Url::from_file_path(&meta.buffile).unwrap(),
        },
        options: params,
        work_done_progress_params: Default::default(),
    };
    ctx.call::<Formatting, _>(
        meta,
        RequestParams::All(vec![req_params]),
        move |ctx, meta, mut result| {
            if let Some((_, result)) = result.pop() {
                let text_edits = result.unwrap_or_default();
                super::range_formatting::editor_range_formatting(meta, text_edits, ctx)
            }
        },
    );
}
