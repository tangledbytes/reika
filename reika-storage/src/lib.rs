use std::{os::fd::RawFd, collections::HashMap, fs};
use reika_reactor::io;

struct IndexEntry {
	file_id: u64,
	value_sz: u32,
	value_pos: u32,
}

pub struct Storage {
	active_file: RawFd,
	directory: RawFd,
	index: HashMap<String, IndexEntry>
}

impl Storage {
	pub async fn init(path: &str) -> Storage {
		io::File::open(path).await.unwrap();

		todo!()
	}
}