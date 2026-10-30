//! 内容摘要计算。
//!
//! 📌【2026-10-30】只有"尺寸相同"的候选才值得读盘算摘要。摘要阶段是整个工具最耗 I/O 的
//! 部分，先用尺寸把候选收缩到最小集合，才能让大目录树的扫描保持可用。

use std::fs::File;
use std::io::{self, Read};
use std::path::PathBuf;

use rayon::prelude::*;
use sha2::{Digest as _, Sha256};

use crate::model::{Digest, FileEntry};

/// 摘要阶段的统计结果。
pub struct DigestOutcome {
    pub hashed: usize,
    /// 读取失败的文件及原因。
    pub failed: Vec<(PathBuf, String)>,
}

const CHUNK_SIZE: usize = 1 << 20;

/// 为指定下标的文件填充内容摘要。
///
/// 只读取磁盘一次：同尺寸候选中内容相同的文件，之后会共享同一份磁盘数据。
pub fn fill_digests(
    entries: &mut [FileEntry],
    candidates: &[usize],
    jobs: usize,
    progress: &indicatif::ProgressBar,
) -> DigestOutcome {
    if candidates.is_empty() {
        return DigestOutcome {
            hashed: 0,
            failed: Vec::new(),
        };
    }

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .build()
        .expect("创建哈希线程池失败");

    let computed: Vec<(usize, Result<Digest, String>)> = pool.install(|| {
        candidates
            .par_iter()
            .map(|&index| {
                let outcome = digest_of(&entries[index].path).map_err(|error| error.to_string());
                progress.inc(1);
                (index, outcome)
            })
            .collect()
    });

    let mut outcome = DigestOutcome {
        hashed: 0,
        failed: Vec::new(),
    };
    for (index, result) in computed {
        match result {
            Ok(digest) => {
                entries[index].digest = Some(digest);
                outcome.hashed += 1;
            }
            Err(reason) => {
                outcome.failed.push((entries[index].path.clone(), reason));
            }
        }
    }
    outcome
}

/// 计算单个文件的 SHA-256 摘要。
///
/// 同时供摘要阶段与硬链接校验阶段使用：校验阶段需要在文件系统不提供文件身份时，
/// 用内容摘要确认两个路径确实指向同一份数据。
pub fn digest_of(path: &std::path::Path) -> io::Result<Digest> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_SIZE];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&hasher.finalize());
    Ok(Digest::from_bytes(bytes))
}
