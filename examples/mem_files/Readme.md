# How to generate random memory file of a given size

## urandom device and dd
``` bash
# Set bs/count to prevent dd to use a huge amount of RAM in case of huge binfile
dd if=/dev/urandom of=rmem_10M.bin bs=10M count=1
```


## urandom and head
``` bash
head -c 10M /dev/urandom > rmem_10M.bin
```
