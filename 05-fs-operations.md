# Using our filesystem

We defined the basic building blocks of our filesystem now we just need to use
them.

So let's go through the basic file operations and see what we need to do.

# Root directory

Before we can do anything we need to establish a baseline. We need to have a
root directory. For now we can just initialize this by hand if we see that
there is none.

For that we need to create an INode with the idx=0. We also need to add two
DirEntries for the "." and ".." directories. And both of them just point back
to the root inode.

Thats it. Now let's get started with creating a file in our root directory.

# Creating a file

To create a file in our root directory we can just create a new INode and
create a DirEntry with the filename and a pointer to the new INode in the root
and thats it. We've created a file in our own filesystem.

# Creating a directory

Very similar process just set the `is_directory` flag on the INode. For now to
keep it simple we can just always attach a '/' to the end of the name of the
filepath.

One thing we didn't account for is that we need some space in the datablocks to
write our DirEntries to somewhere. 

For that we need a simple free list. We need some way of storing which of the
blocks is used and which one is free. For that we can use a simple bitmap as we
already defined the number of available INode blocks and DataBlocks.

With this in place we know which one of the blocks we can use to write the
DirEntries.

But what happens after we restart and reload. Right now this is not a problem
because we're just using a ramdisk which does not persist anyways but later we
want to use a real disk and then we need to know which of the blocks are
used/free.

If only we had a place to write metadata to write in our filesystem. The
`SuperBlock`. We can add this information to our superblock.

Now it's a bit more complicated as we have a couple of growing things in our
superblock now. 

To abstract this away, let's define a Layout. This defines which blocks in the
memory are used for what. Now we need blocks to store the bitmaps for the Data
and Inode blocks and also the actual inode blocks and data blocks. We can use
this layout to use it as offsets for indexing into the arrays.

```
+------------+
| Superblock |
+------------+
|   INodes   |
+------------+
|    Data    |
|     ..     |
|     ..     |
|     ..     |
+------------+
```

We haven't covered a couple of topics such as removing files or deleting
content from a file. But this will come later ...




