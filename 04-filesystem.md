# Filesystem - How to organize a block of memory into usable files

Before we start by adding a real disk as a persistant storage to our OS we can
make our life simple by using a Ramdisk for now. The interface of `read_block`
and `write_block` will be the same anyways and a ramdisk let's us test the
logic first without persistant storage.

For the ramdisk we can just create a big block of static memory.

Now we have a big blob of memory. But how do we represent and address files in
that blob.

First we need to define a structure to identify our filesystem and make sure
it's a valid instance.

# Superblock

A `Superblock` contains metadata and general information about our filesystem.
One piece of this metadata can be a magic value. When we read data from this
filesystem during initalization we can read the magic and know if this is a
filesystem that we know about or not.

We will add more and more information to the Superblock as we go on
implementing the filesystem.

# INode

We need a way to organize our files and directories. To start with we can say
that directories are just files with a `is_directory` flag set. Other than that
they behave in a same way for us so far.

To store information about our files (not the data itself) we allocate a couple
of pages at the beginning of our big blob of memory. We can use those blocks as
an array of `INode`s. Each of the INodes is describing a file.

For now we can think of the inode as a descriptor of a file. It contains flags
such as our `is_directory` flag, the size of the data and pointers to the
data blocks of this file.

Later on we might add other flags to it, such as permissions etc. But for now
this is enough.

# Data

INodes are the descriptors of files but where does the actual content of a file
or a directory live?

The data also has to be stored somewhere in the big blob of memory. We've
already allocate the first couple of blocks to store INodes and we don't really
need anything else for now, so we can just use the rest of the available blocks
as datablocks.

Now I already told you that our INode is holding some pointers to the data blocks.
Those are basically just indexes into our array of data blocks that is right after
our array of INode blocks in the big blob of memory.

An important thing to mention here is that a block always has to be assigned to
a single file. It's not possible for a block to contain data from multiple
files.

Now the question is how do we represent the data of files (or directories) in our
data blocks.

This is the only other place where the handling is different between files and
directories. For files that is quite simple as we just write the content of the
file to a block and thats it.

For directories we write `DirEntry`s to the datablock. You might have noticed that
the inode doesn't contain the name of the file. This is where the DirEntry comes
to play. A DirEntry consist of a pointer to an INode and a fixed size name. For
this OS we can choose something relatively small as it reduces the size of the
DirEntry.

And thats our basic building blocks for our filesystem.
