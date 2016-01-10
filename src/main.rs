extern crate fuse;
extern crate libc;
extern crate time;
extern crate git2;

use std::env;
use std::path::Path;
use std::collections::HashMap;
use std::cmp::max;

use libc::ENOENT;
use time::Timespec;

use fuse::{FileType, FileAttr, Filesystem, Request, ReplyData, ReplyEntry, ReplyAttr, ReplyDirectory};

use git2::{Repository, Tree, Blob, Object, Oid, TreeEntry, ObjectType};

const TTL: Timespec = Timespec { sec: 1, nsec: 0 };                 // 1 second

const CREATE_TIME: Timespec = Timespec { sec: 1381237736, nsec: 0 };    // 2013-10-08 08:56

struct Bimap {
    forward: Vec<Oid>,
    reverse: HashMap<Oid, usize>
}

impl Bimap {
    fn new() -> Bimap {
        Bimap {
            forward: Vec::new(),
            reverse: HashMap::new()
        }
    }

    fn get_forward(&self, k: u64) -> Option<Oid> {
        if (k as usize) <= self.forward.len() {
            Some(self.forward[k as usize - 1])
        } else {
            None
        }
    }

    fn get_reverse(&self, v: &Oid) -> Option<u64> {
        match self.reverse.get(v) {
            Some(&k) => Some(k as u64),
            None => None
        }
    }

    fn get_reverse_or_alloc(&mut self, v: &Oid) -> u64 {
        match self.get_reverse(v) {
            Some(k) => return k,
            None => {}
        }


        self.forward.push(*v);

        let k = self.forward.len();

        self.reverse.insert(*v, k);

        k as u64
    }
}

struct GitFilesystem {
    repo: Repository,
    nodes: Bimap
}

fn get_tree_entry_info<'repo, 'entry>(
    nodes: &mut Bimap,
    entry: &'entry TreeEntry<'repo>) -> (u64, FileType, &'entry str) {

    let kind = match entry.kind().unwrap() {
        ObjectType::Tree => FileType::Directory,
        ObjectType::Blob => FileType::RegularFile,
        t => panic!("unexpected type: {:?}", t)
    };

    let name = entry.name().unwrap();

    (nodes.get_reverse_or_alloc(&entry.id()), kind, name)
}

fn get_tree<'repo>(repo: &'repo Repository, nodes: &mut Bimap, ino: u64) -> Result<Tree<'repo>, git2::Error> {
    let oid = match nodes.get_forward(ino) {
        Some(v) => v,
        None => return Err(git2::Error::from_str("inode not found"))
    };
    repo.find_tree(oid)
}

fn get_obj<'repo>(repo: &'repo Repository, nodes: &mut Bimap, ino: u64) -> Result<Object<'repo>, git2::Error> {
    let oid = match nodes.get_forward(ino) {
        Some(v) => v,
        None => return Err(git2::Error::from_str("inode not found"))
    };
    repo.find_object(oid, None)
}

impl GitFilesystem {
    fn new(repo: Repository, root: Oid) -> GitFilesystem {
        let mut g = GitFilesystem {
            repo: repo,
            nodes: Bimap::new()
        };

        g.nodes.forward.push(root);
        g.nodes.reverse.insert(root, 1);

        g
    }
}

impl Filesystem for GitFilesystem {
    fn lookup (&mut self, _req: &Request, parent: u64, name: &Path, reply: ReplyEntry) {
        println!("lookup {:?} {:?}", parent, name);

        let tree = get_tree(&self.repo, &mut self.nodes, parent);

        match tree {
             Ok(tree) => {
                for i in 0..tree.len() {
                    let entry = tree.get(i).unwrap();

                    if entry.name().unwrap() == name.to_str().unwrap() {
                        let (ino, kind, name) = get_tree_entry_info(&mut self.nodes, &entry);

                        let obj = self.repo.find_object(entry.id(), None).unwrap();

                        let (kind, size) = if let Some(blob) = obj.as_blob() {
                            (FileType::RegularFile, blob.content().len())
                        } else {
                            match obj.kind().unwrap() {
                                ObjectType::Tree => (FileType::Directory, 0),
                                t => panic!("unexpected type: {:?}", t)
                            }
                        };

                        let attr = FileAttr {
                            ino: ino,
                            size: size as u64,
                            blocks: (size + 4095) as u64 / 4096,
                            atime: CREATE_TIME,
                            mtime: CREATE_TIME,
                            ctime: CREATE_TIME,
                            crtime: CREATE_TIME,
                            kind: kind,
                            perm: 0o755,
                            nlink: 2,
                            uid: 99,
                            gid: 99,
                            rdev: 0,
                            flags: 0,
                        };

                        println!("  entry {:?}", attr);
                        reply.entry(&TTL, &attr, 0);
                        return;
                    }
                }
            }
            Err(e) => {
                println!("error: {:?}", e);
            }
        }

        reply.error(ENOENT);
    }

    fn getattr (&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        println!("getattr {:?}", ino);

        let obj = match get_obj(&mut self.repo, &mut self.nodes, ino) {
            Ok(obj) => obj,
            Err(e) => {
                panic!("object not found; error: {:?}", e);
            }
        };

        let (kind, size) = if let Some(blob) = obj.as_blob() {
            (FileType::RegularFile, blob.content().len())
        } else {
            match obj.kind().unwrap() {
                ObjectType::Tree => (FileType::Directory, 0),
                t => panic!("unexpected type: {:?}", t)
            }
        };

        let attr = FileAttr {
            ino: ino,
            size: size as u64,
            blocks: (size + 4095) as u64 / 4096,
            atime: CREATE_TIME,
            mtime: CREATE_TIME,
            ctime: CREATE_TIME,
            crtime: CREATE_TIME,
            kind: kind,
            perm: 0o755,
            nlink: 2,
            uid: 99,
            gid: 99,
            rdev: 0,
            flags: 0,
        };

        println!("  attr {:?}", attr);
        reply.attr(&TTL, &attr);

        // match ino {
        //     1 => reply.attr(&TTL, &HELLO_DIR_ATTR),
        //     2 => reply.attr(&TTL, &HELLO_TXT_ATTR),
        //     _ => reply.error(ENOENT),
        // }
    }

    fn read (&mut self, _req: &Request, ino: u64, _fh: u64, offset: u64, _size: u32, reply: ReplyData) {
        println!("read {:?} {:?} {:?} {:?}", ino, _fh, offset, _size);

        let obj = match get_obj(&mut self.repo, &mut self.nodes, ino) {
            Ok(obj) => obj,
            Err(e) => {
                panic!("object not found; error: {:?}", e);
            }
        };

        if let Some(blob) = obj.as_blob() {
            reply.data(&blob.content()[offset as usize .. offset as usize + _size as usize]);
        } else {
            panic!("unexpected type: {:?}", obj.kind())
        };

    }

    fn readdir (&mut self, _req: &Request, ino: u64, _fh: u64, offset: u64, mut reply: ReplyDirectory) {
        println!("readdir {:?} {:?} {:?}", ino, _fh, offset);

        let tree = get_tree(&mut self.repo, &mut self.nodes, ino);

        match tree {
             Ok(tree) => {
                if offset != 0 && offset as usize != tree.len() + 1 {
                    panic!("unexpected offset: {}", offset);
                }

                if offset == 0 {
                    println!("  add 1 0 Directory .");
                    reply.add(1, 0, FileType::Directory, ".");
                    println!("  add 1 1 Directory ..");
                    reply.add(1, 1, FileType::Directory, "..");

                    for i in 0..tree.len() {
                        let entry = tree.get(i).unwrap();
                        let (ino, kind, name) = get_tree_entry_info(&mut self.nodes, &entry);

                        println!("  add {} {} {:?} {}", ino, i + 2, kind, name);
                        reply.add(ino, i as u64 + 2, kind, name);
                    }
                }

                reply.ok();
                return;
            }
            Err(e) => {
                println!("error: {:?}", e);
            }
        }

        reply.error(ENOENT);
    }
}

fn main () {

    let repo = match Repository::open(".") {
        Ok(repo) => repo,
        Err(e) => panic!("failed to open: {}", e),
    };

    let master = repo.revparse_single("master").unwrap().id();

    let tree = repo.find_commit(master).unwrap().tree().unwrap().id();

    let mountpoint = env::args_os().nth(1).unwrap();
    fuse::mount(GitFilesystem::new(repo, tree), &mountpoint, &[]);
}
