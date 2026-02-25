# Filesystem

We eventually want to be able to use a filesystem in our OS. Since using a real
filesystem from inside QEMU is quite tricky we'll start by using a ramdisk which
lives in memory inside of our binary.

## Ramdisk

To create a `Ramdisk` we just allocate a static array with a reasonable size.
Let's choose 1MB.

First we need to define how the filesystem should be represented in memory.

Usually the memory is split into blocks. We can then either read or write those
blocks. A common size to choose for a block is 512 bytes.

Now we can define a Layout of what blocks mean what. This metadata is usually
written to a special block at `BlockIndex(0)` in memory called a `SuperBlock`.

## Superblock

The `SuperBlock` starts with a magic value that we can read to know that the
following memory is following our layout.

For the sake of this project we'll be using `0x4e4f4d454c` as our magic value
as it spells out `lemon`.

We already created the first piece of our layout with the superblock. Now we
can define the rest of the blocks. The question is what kind of other blocks
do we need to have a fully working filesystem.

## INode
An `INode` is a descriptor that holds information about an entry in the
filesystem. This can be a file or a directory.

For now, we can store the information wether this is a file or a directory,
it's content and it's size.

The should be enough to get started.

## Data blocks
The `INode` points to some data blocks. This data has to be interpreted
differently depending on the `is_directory` flag on the `INode`. If this flag
is true, we need to read those as directory entries. If not then it's just the
raw content of the file.

## DirEntry
A DirEntry is the structure that is written to the data blocks that a directory
`INode` points to. For now it just contains a name and points to the inode for
this object.

## Layout
Now we have all the building blocks we need to know to bulid a basic
filesystem!

We can define out layout as such 

```
+------------+
| Superblock |
+------------+
|   INodes   |
+------------+
|    Data    |
+------------+
```

## How does this work now.

Each entry in the root (`/`) directory has a `DirEntry` in the root data blocks.
Each of those contains a pointer to an `INode` and a name. The `INode` contains
the metadata about this entry for example if it's a file or a directory.

Now we have enough to create files and directories. To write to a file, we just
need to write to it's data blocks.

This means we can write a simple `ls` or `tree` command in our shell.

## Deletion ...

## Flushing ...
