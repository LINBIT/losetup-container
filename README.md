# losetup-container

Normal `losetup`, with one modification. If, and only if it is called with `losetup -l -O NAME,BACK-FILE`, the
output is slightly different, in all other cases the normal `losetup` output is used.

## Problem with `losetup`

What's wrong with the normal `losetup`? This is difficult to put into words, so here is an example:

```
# fallocate -l 100m example
# docker run --privileged --rm -v $PWD:/foo -w /foo -v /dev:/dev almalinux losetup --show -f /foo/example
/dev/loop0
# losetup -l
NAME       SIZELIMIT OFFSET AUTOCLEAR RO BACK-FILE DIO LOG-SEC
/dev/loop0         0      0         0  0 /example    0     512
```

Note that the reported backing file is now `/example`. This is because the loop device only reports the path starting
from the original mount point of our file. Since we bound our working directory to `/foo`, we only see `/example`.

This is an issue for [LINSTOR®](https://github.com/linbit/linstor-server), and it's file backed storage pools.
If run inside a container, and that container restarts, the satellite will think the files are no longer mounted, which
then triggers all kinds of unnecessary updates to the volume configuration.

## The solution

If `losetup -l -O NAME,BACK-FILE` is executed, which is the exact command executed by LINSTOR, `losetup-container` will
try to find the right backing file. It does that as follows:

* List all loop devices by searching `/sys/block` for directories starting with `loop`
* For each `loopX` directory:
  * Run `ioctl(/dev/loopX, LOOP_GET_STATUS64, ...)`, getting the original backing file name and inode. Since the
    original file name is limited to 64 characters, it might have been truncated. Add it to the list of candidates.
  * Read `/sys/block/loopX/loop/backing_file`, which contains the backing file, potentially truncated to the path to
    the last mount point, as seen in the example above. Add it to the list of candidates.
  * Read the `LOSETUP_CONTAINER_BIND_MOUNTS` variable. It contains a newline delimited list of paths that will be
    checked. For each entry, join the path from `loop/backing_file`, adding it to the list of candidates.
  * Check each candidate, if it exists, and the inode of the file matches the reported inode from the `ioctl()`, we
    found the current path of the backing file.
  * If no candidate matched, we report the file from the sys fs `loop/backing_file`, like `losetup`

If the arguments do not match exactly, `losetup-container` will delegate to the program specified in
`LOSETUP_CONTAINER_ORIGINAL_LOSETUP` or `/usr/sbin/losetup`. This should be the original `losetup` binary.
