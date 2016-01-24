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

use fuse::{
    FileType,
    FileAttr,
    Filesystem,
    Request,
    ReplyData,
    ReplyEntry,
    ReplyAttr,
    ReplyEmpty,
    ReplyOpen,
    ReplyWrite,
    ReplyStatfs,
    ReplyCreate,
    ReplyLock,
    ReplyBmap,
    ReplyXTimes,
    ReplyDirectory
};

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
        // println!("lookup {:?} {:?}", parent, name);

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

                        // println!("  entry {:?}", attr);
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
        // println!("getattr {:?}", ino);

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

        // println!("  attr {:?}", attr);
        reply.attr(&TTL, &attr);

        // match ino {
        //     1 => reply.attr(&TTL, &HELLO_DIR_ATTR),
        //     2 => reply.attr(&TTL, &HELLO_TXT_ATTR),
        //     _ => reply.error(ENOENT),
        // }
    }

    fn read (&mut self, _req: &Request, ino: u64, _fh: u64, offset: u64, _size: u32, reply: ReplyData) {
        // println!("read {:?} {:?} {:?} {:?}", ino, _fh, offset, _size);

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
        // println!("readdir {:?} {:?} {:?}", ino, _fh, offset);

        let tree = get_tree(&mut self.repo, &mut self.nodes, ino);

        match tree {
             Ok(tree) => {
                if offset != 0 && offset as usize != tree.len() + 1 {
                    panic!("unexpected offset: {}", offset);
                }

                if offset == 0 {
                    // println!("  add 1 0 Directory .");
                    reply.add(1, 0, FileType::Directory, ".");
                    // println!("  add 1 1 Directory ..");
                    reply.add(1, 1, FileType::Directory, "..");

                    for i in 0..tree.len() {
                        let entry = tree.get(i).unwrap();
                        let (ino, kind, name) = get_tree_entry_info(&mut self.nodes, &entry);

                        // println!("  add {} {} {:?} {}", ino, i + 2, kind, name);
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

struct LoggingFilesystem<T: Filesystem> {
    inner: T
}

impl<T: Filesystem> LoggingFilesystem<T> {
    fn new(inner: T) -> LoggingFilesystem<T> {
        LoggingFilesystem {
            inner: inner
        }
    }
}

impl<T: Filesystem> Filesystem for LoggingFilesystem<T> {
    /// Initialize filesystem
    /// Called before any other filesystem method.
    fn init (&mut self, _req: &Request) -> Result<(), libc::c_int> {
        let res = self.inner.init(_req);
        println!("self.inner.init() -> {:?}", res);
        res
    }

    /// Clean up filesystem
    /// Called on filesystem exit.
    fn destroy (&mut self, _req: &Request) {
        self.inner.destroy(_req);
        println!("self.inner.destroy()");
    }

    /// Look up a directory entry by name and get its attributes.
    fn lookup (&mut self, _req: &Request, _parent: u64, _name: &Path, reply: ReplyEntry) {
        self.inner.lookup(_req, _parent, _name, reply);
        println!("self.inner.lookup({}, {:?})", _parent, _name);
    }

    /// Forget about an inode
    /// The nlookup parameter indicates the number of lookups previously performed on
    /// this inode. If the filesystem implements inode lifetimes, it is recommended that
    /// inodes acquire a single reference on each lookup, and lose nlookup references on
    /// each forget. The filesystem may ignore forget calls, if the inodes don't need to
    /// have a limited lifetime. On unmount it is not guaranteed, that all referenced
    /// inodes will receive a forget message.
    fn forget (&mut self, _req: &Request, _ino: u64, _nlookup: u64) {
        self.inner.forget(_req, _ino, _nlookup);
        println!("self.inner.forget({})", _ino);
    }

    /// Get file attributes
    fn getattr (&mut self, _req: &Request, _ino: u64, reply: ReplyAttr) {
        self.inner.getattr(_req, _ino, reply);
        println!("self.inner.getattr({})", _ino);
    }

    /// Set file attributes
    fn setattr (&mut self, _req: &Request, _ino: u64, _mode: Option<u32>, _uid: Option<u32>, _gid: Option<u32>, _size: Option<u64>, _atime: Option<Timespec>, _mtime: Option<Timespec>, _fh: Option<u64>, _crtime: Option<Timespec>, _chgtime: Option<Timespec>, _bkuptime: Option<Timespec>, _flags: Option<u32>, reply: ReplyAttr) {
        self.inner.setattr(_req, _ino, _mode, _uid, _gid, _size, _atime, _mtime, _fh, _crtime, _chgtime, _bkuptime, _flags, reply);
        println!("self.inner.setattr({}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?}, {:?})", _ino, _mode, _uid, _gid, _size, _atime, _mtime, _fh, _crtime, _chgtime, _bkuptime, _flags);
    }

    /// Read symbolic link
    fn readlink (&mut self, _req: &Request, _ino: u64, reply: ReplyData) {
        self.inner.readlink(_req, _ino, reply);
        println!("self.inner.readlink({})", _ino);
    }

    /// Create file node
    /// Create a regular file, character device, block device, fifo or socket node.
    fn mknod (&mut self, _req: &Request, _parent: u64, _name: &Path, _mode: u32, _rdev: u32, reply: ReplyEntry) {
        self.inner.mknod(_req, _parent, _name, _mode, _rdev, reply);
        println!("self.inner.mknod({}, {:?}, {}, {})", _parent, _name, _mode, _rdev);
    }

    /// Create a directory
    fn mkdir (&mut self, _req: &Request, _parent: u64, _name: &Path, _mode: u32, reply: ReplyEntry) {
        self.inner.mkdir(_req, _parent, _name, _mode, reply);
        println!("self.inner.mkdir({}, {:?}, {})", _parent, _name, _mode);
    }

    /// Remove a file
    fn unlink (&mut self, _req: &Request, _parent: u64, _name: &Path, reply: ReplyEmpty) {
        self.inner.unlink(_req, _parent, _name, reply);
        println!("self.inner.unlink({}, {:?})", _parent, _name);
    }

    /// Remove a directory
    fn rmdir (&mut self, _req: &Request, _parent: u64, _name: &Path, reply: ReplyEmpty) {
        self.inner.rmdir(_req, _parent, _name, reply);
        println!("self.inner.rmdir({}, {:?})", _parent, _name);
    }

    /// Create a symbolic link
    fn symlink (&mut self, _req: &Request, _parent: u64, _name: &Path, _link: &Path, reply: ReplyEntry) {
        self.inner.symlink(_req, _parent, _name, _link, reply);
        println!("self.inner.symlink({}, {:?}, {:?})", _parent, _name, _link);
    }

    /// Rename a file
    fn rename (&mut self, _req: &Request, _parent: u64, _name: &Path, _newparent: u64, _newname: &Path, reply: ReplyEmpty) {
        self.inner.rename(_req, _parent, _name, _newparent, _newname, reply);
        println!("self.inner.rename({}, {:?}, {}, {:?})", _parent, _name, _newparent, _newname);
    }

    /// Create a hard link
    fn link (&mut self, _req: &Request, _ino: u64, _newparent: u64, _newname: &Path, reply: ReplyEntry) {
        self.inner.link(_req, _ino, _newparent, _newname, reply);
        println!("self.inner.link({}, {}, {:?})", _ino, _newparent, _newname);
    }

    /// Open a file
    /// Open flags (with the exception of O_CREAT, O_EXCL, O_NOCTTY and O_TRUNC) are
    /// available in flags. Filesystem may store an arbitrary file handle (pointer, index,
    /// etc) in fh, and use this in other all other file operations (read, write, flush,
    /// release, fsync). Filesystem may also implement stateless file I/O and not store
    /// anything in fh. There are also some flags (direct_io, keep_cache) which the
    /// filesystem may set, to change the way the file is opened. See fuse_file_info
    /// structure in <fuse_common.h> for more details.
    fn open (&mut self, _req: &Request, _ino: u64, _flags: u32, reply: ReplyOpen) {
        self.inner.open(_req, _ino, _flags, reply);
        println!("self.inner.open({}, {:?})", _ino, _flags);
    }

    /// Read data
    /// Read should send exactly the number of bytes requested except on EOF or error,
    /// otherwise the rest of the data will be substituted with zeroes. An exception to
    /// this is when the file has been opened in 'direct_io' mode, in which case the
    /// return value of the read system call will reflect the return value of this
    /// operation. fh will contain the value set by the open method, or will be undefined
    /// if the open method didn't set any value.
    fn read (&mut self, _req: &Request, _ino: u64, _fh: u64, _offset: u64, _size: u32, reply: ReplyData) {
        self.inner.read(_req, _ino, _fh, _offset, _size, reply);
        println!("self.inner.read({}, {}, {}, {})", _ino, _fh, _offset, _size);
    }

    /// Write data
    /// Write should return exactly the number of bytes requested except on error. An
    /// exception to this is when the file has been opened in 'direct_io' mode, in
    /// which case the return value of the write system call will reflect the return
    /// value of this operation. fh will contain the value set by the open method, or
    /// will be undefined if the open method didn't set any value.
    fn write (&mut self, _req: &Request, _ino: u64, _fh: u64, _offset: u64, _data: &[u8], _flags: u32, reply: ReplyWrite) {
        self.inner.write(_req, _ino, _fh, _offset, _data, _flags, reply);
        println!("self.inner.write({}, {}, {}, len: {}, {})", _ino, _fh, _offset, _data.len(), _flags);
    }

    /// Flush method
    /// This is called on each close() of the opened file. Since file descriptors can
    /// be duplicated (dup, dup2, fork), for one open call there may be many flush
    /// calls. Filesystems shouldn't assume that flush will always be called after some
    /// writes, or that if will be called at all. fh will contain the value set by the
    /// open method, or will be undefined if the open method didn't set any value.
    /// NOTE: the name of the method is misleading, since (unlike fsync) the filesystem
    /// is not forced to flush pending writes. One reason to flush data, is if the
    /// filesystem wants to return write errors. If the filesystem supports file locking
    /// operations (setlk, getlk) it should remove all locks belonging to 'lock_owner'.
    fn flush (&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, reply: ReplyEmpty) {
        self.inner.flush(_req, _ino, _fh, _lock_owner, reply);
        println!("self.inner.flush({}, {}, {})", _ino, _fh, _lock_owner);
    }

    /// Release an open file
    /// Release is called when there are no more references to an open file: all file
    /// descriptors are closed and all memory mappings are unmapped. For every open
    /// call there will be exactly one release call. The filesystem may reply with an
    /// error, but error values are not returned to close() or munmap() which triggered
    /// the release. fh will contain the value set by the open method, or will be undefined
    /// if the open method didn't set any value. flags will contain the same flags as for
    /// open.
    fn release (&mut self, _req: &Request, _ino: u64, _fh: u64, _flags: u32, _lock_owner: u64, _flush: bool, reply: ReplyEmpty) {
        self.inner.release(_req, _ino, _fh, _flags, _lock_owner, _flush, reply);
        println!("self.inner.release({}, {}, {}, {}, {})", _ino, _fh, _flags, _lock_owner, _flush);
    }

    /// Synchronize file contents
    /// If the datasync parameter is non-zero, then only the user data should be flushed,
    /// not the meta data.
    fn fsync (&mut self, _req: &Request, _ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) {
        self.inner.fsync(_req, _ino, _fh, _datasync, reply);
        println!("self.inner.fsync({}, {}, {})", _ino, _fh, _datasync);
    }

    /// Open a directory
    /// Filesystem may store an arbitrary file handle (pointer, index, etc) in fh, and
    /// use this in other all other directory stream operations (readdir, releasedir,
    /// fsyncdir). Filesystem may also implement stateless directory I/O and not store
    /// anything in fh, though that makes it impossible to implement standard conforming
    /// directory stream operations in case the contents of the directory can change
    /// between opendir and releasedir.
    fn opendir (&mut self, _req: &Request, _ino: u64, _flags: u32, reply: ReplyOpen) {
        self.inner.opendir(_req, _ino, _flags, reply);
        println!("self.inner.opendir({}, {})", _ino, _flags);
    }

    /// Read directory
    /// Send a buffer filled using buffer.fill(), with size not exceeding the
    /// requested size. Send an empty buffer on end of stream. fh will contain the
    /// value set by the opendir method, or will be undefined if the opendir method
    /// didn't set any value.
    fn readdir (&mut self, _req: &Request, _ino: u64, _fh: u64, _offset: u64, reply: ReplyDirectory) {
        self.inner.readdir(_req, _ino, _fh, _offset, reply);
        println!("self.inner.readdir({}, {}, {})", _ino, _fh, _offset);
    }

    /// Release an open directory
    /// For every opendir call there will be exactly one releasedir call. fh will
    /// contain the value set by the opendir method, or will be undefined if the
    /// opendir method didn't set any value.
    fn releasedir (&mut self, _req: &Request, _ino: u64, _fh: u64, _flags: u32, reply: ReplyEmpty) {
        self.inner.releasedir(_req, _ino, _fh, _flags, reply);
        println!("self.inner.releasedir({}, {}, {})", _ino, _fh, _flags);
    }

    /// Synchronize directory contents
    /// If the datasync parameter is set, then only the directory contents should
    /// be flushed, not the meta data. fh will contain the value set by the opendir
    /// method, or will be undefined if the opendir method didn't set any value.
    fn fsyncdir (&mut self, _req: &Request, _ino: u64, _fh: u64, _datasync: bool, reply: ReplyEmpty) {
        self.inner.fsyncdir(_req, _ino, _fh, _datasync, reply);
        println!("self.inner.fsyncdir({}, {}, {})", _ino, _fh, _datasync);
    }

    /// Get file system statistics
    fn statfs (&mut self, _req: &Request, _ino: u64, reply: ReplyStatfs) {
        self.inner.statfs(_req, _ino, reply);
        println!("self.inner.statfs({})", _ino);
    }

    /// Set an extended attribute
    fn setxattr (&mut self, _req: &Request, _ino: u64, _name: &std::ffi::OsStr, _value: &[u8], _flags: u32, _position: u32, reply: ReplyEmpty) {
        self.inner.setxattr(_req, _ino, _name, _value, _flags, _position, reply);
        println!("self.inner.setxattr({}, {:?}, len: {}, {}, {})", _ino, _name, _value.len(), _flags, _position);
    }

    /// Get an extended attribute
    fn getxattr (&mut self, _req: &Request, _ino: u64, _name: &std::ffi::OsStr, reply: ReplyData) {
        self.inner.getxattr(_req, _ino, _name, reply);
        println!("self.inner.getxattr({}, {:?})", _ino, _name);
    }

    /// List extended attribute names
    fn listxattr (&mut self, _req: &Request, _ino: u64, reply: ReplyEmpty) {
        self.inner.listxattr(_req, _ino, reply);
        println!("self.inner.listxattr({})", _ino);
    }

    /// Remove an extended attribute
    fn removexattr (&mut self, _req: &Request, _ino: u64, _name: &std::ffi::OsStr, reply: ReplyEmpty) {
        self.inner.removexattr(_req, _ino, _name, reply);
        println!("self.inner.removexattr({}, {:?})", _ino, _name);
    }

    /// Check file access permissions
    /// This will be called for the access() system call. If the 'default_permissions'
    /// mount option is given, this method is not called. This method is not called
    /// under Linux kernel versions 2.4.x
    fn access (&mut self, _req: &Request, _ino: u64, _mask: u32, reply: ReplyEmpty) {
        self.inner.access(_req, _ino, _mask, reply);
        println!("self.inner.access({}, {})", _ino, _mask);
    }

    /// Create and open a file
    /// If the file does not exist, first create it with the specified mode, and then
    /// open it. Open flags (with the exception of O_NOCTTY) are available in flags.
    /// Filesystem may store an arbitrary file handle (pointer, index, etc) in fh,
    /// and use this in other all other file operations (read, write, flush, release,
    /// fsync). There are also some flags (direct_io, keep_cache) which the
    /// filesystem may set, to change the way the file is opened. See fuse_file_info
    /// structure in <fuse_common.h> for more details. If this method is not
    /// implemented or under Linux kernel versions earlier than 2.6.15, the mknod()
    /// and open() methods will be called instead.
    fn create (&mut self, _req: &Request, _parent: u64, _name: &Path, _mode: u32, _flags: u32, reply: ReplyCreate) {
        self.inner.create(_req, _parent, _name, _mode, _flags, reply);
        println!("self.inner.create({}, {:?}, {}, {})", _parent, _name, _mode, _flags);
    }

    /// Test for a POSIX file lock
    fn getlk (&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, _start: u64, _end: u64, _typ: u32, _pid: u32, reply: ReplyLock) {
        self.inner.getlk(_req, _ino, _fh, _lock_owner, _start, _end, _typ, _pid, reply);
        println!("self.inner.getlk({}, {}, {}, {}, {}, {}, {})", _ino, _fh, _lock_owner, _start, _end, _typ, _pid);
    }

    /// Acquire, modify or release a POSIX file lock
    /// For POSIX threads (NPTL) there's a 1-1 relation between pid and owner, but
    /// otherwise this is not always the case.  For checking lock ownership,
    /// 'fi->owner' must be used. The l_pid field in 'struct flock' should only be
    /// used to fill in this field in getlk(). Note: if the locking methods are not
    /// implemented, the kernel will still allow file locking to work locally.
    /// Hence these are only interesting for network filesystems and similar.
    fn setlk (&mut self, _req: &Request, _ino: u64, _fh: u64, _lock_owner: u64, _start: u64, _end: u64, _typ: u32, _pid: u32, _sleep: bool, reply: ReplyEmpty) {
        self.inner.setlk(_req, _ino, _fh, _lock_owner, _start, _end, _typ, _pid, _sleep, reply);
        println!("self.inner.setlk({}, {}, {}, {}, {}, {}, {}, {})", _ino, _fh, _lock_owner, _start, _end, _typ, _pid, _sleep);
    }

    /// Map block index within file to block index within device
    /// Note: This makes sense only for block device backed filesystems mounted
    /// with the 'blkdev' option
    fn bmap (&mut self, _req: &Request, _ino: u64, _blocksize: u32, _idx: u64, reply: ReplyBmap) {
        self.inner.bmap(_req, _ino, _blocksize, _idx, reply);
        println!("self.inner.bmap({}, {}, {})", _ino, _blocksize, _idx);
    }

    /// OS X only: Rename the volume. Set fuse_init_out.flags during init to
    /// FUSE_VOL_RENAME to enable
    #[cfg(target_os = "macos")]
    fn setvolname (&mut self, _req: &Request, _name: &std::ffi::OsStr, reply: ReplyEmpty) {
        self.inner.setvolname(_req, _name, reply);
        println!("self.inner.setvolname({:?})", _name);
    }

    /// OS X only (undocumented)
    #[cfg(target_os = "macos")]
    fn exchange (&mut self, _req: &Request, _parent: u64, _name: &Path, _newparent: u64, _newname: &Path, _options: u64, reply: ReplyEmpty) {
        self.inner.exchange(_req, _parent, _name, _newparent, _newname, _options, reply);
        println!("self.inner.exchange({}, {:?}, {}, {:?}, {})", _parent, _name, _newparent, _newname, _options);
    }

    /// OS X only: Query extended times (bkuptime and crtime). Set fuse_init_out.flags
    /// during init to FUSE_XTIMES to enable
    #[cfg(target_os = "macos")]
    fn getxtimes (&mut self, _req: &Request, _ino: u64, reply: ReplyXTimes) {
        self.inner.getxtimes(_req, _ino, reply);
        println!("self.inner.getxtimes({})", _ino);
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
    fuse::mount(LoggingFilesystem::new(GitFilesystem::new(repo, tree)), &mountpoint, &[]);
}
