/*
 * Copyright © 2020-present Peter M. Stahl pemistahl@gmail.com
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either expressed or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */


// yomu での変更: 言語ごとの統計データ (*.fst) をバイナリに埋め込まず、set_models_directory で設定した
// ディレクトリ (言語の ISO 639-1 のコードごとのディレクトリ) から読む。データを読めない言語は、元の lingua で
// その言語の機能を有効にしていないときと同じく、統計を使わずに判定する

use crate::Language;
use crate::detector::{CountModelFst, LanguageModelFst};
use std::borrow::Cow;
use std::fs::File;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static MODELS_DIRECTORY: OnceLock<PathBuf> = OnceLock::new();

/// 統計データの置き場所を設定する。<dir>/<ISO 639-1 のコード>/ngrams.fst などを読む。最初の 1 回だけ有効
pub fn set_models_directory(dir: &Path) {
    let _ = MODELS_DIRECTORY.set(dir.to_path_buf());
}

fn language_path(language: Language, file_name: &str) -> std::io::Result<PathBuf> {
    let dir = MODELS_DIRECTORY.get().ok_or(ErrorKind::NotFound)?;
    Ok(dir.join(language.iso_code_639_1().to_string()).join(file_name))
}

/// ファイルをメモリに対応づけて読む。読んだモデルは lingua がずっと持つ (元の lingua でも 'static) ので、対応づけは解かない
fn map_file(path: &Path) -> std::io::Result<&'static [u8]> {
    let mmap = unsafe { memmap2::Mmap::map(&File::open(path)?)? };
    Ok(&**Box::leak(Box::new(mmap)))
}

pub(crate) fn read_probability_model_data_file(
    language: Language,
    file_name: &str,
) -> std::io::Result<LanguageModelFst> {
    let bytes = map_file(&language_path(language, file_name)?)?;
    fst::Map::new(Cow::Borrowed(bytes)).map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))
}

pub(crate) fn read_count_model_data_file(
    language: Language,
    file_name: &str,
) -> std::io::Result<CountModelFst> {
    let bytes = map_file(&language_path(language, file_name)?)?;
    fst::Set::new(Cow::Borrowed(bytes)).map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))
}

/// 学習用の文 (モデルを作り直すときだけ使う)。<dir>/<コード>/testdata/ から読む
pub(crate) fn read_test_data_file(language: Language, file_name: &str) -> std::io::Result<&'static str> {
    let text = std::fs::read_to_string(language_path(language, &format!("testdata/{file_name}"))?)?;
    Ok(Box::leak(text.into_boxed_str()))
}
