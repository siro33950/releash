use std::path::PathBuf;
use std::process::Output;

use crate::infrastructure::process::output::{output, ProcessError};

pub(crate) async fn move_to_trash(paths: &[PathBuf]) -> Result<Output, ProcessError> {
    let mut command = tokio::process::Command::new("osascript");
    command.args(["-l", "JavaScript", "-e", "ObjC.import('Foundation'); function run(paths) { for (const path of paths) { const error = Ref(); if (!$.NSFileManager.defaultManager.trashItemAtURLResultingItemURLError($.NSURL.fileURLWithPath(path), null, error)) throw Error(ObjC.unwrap(error[0].localizedDescription)); } }"]);
    command.args(paths);
    output(command, Vec::new()).await
}
