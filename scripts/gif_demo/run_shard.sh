#!/bin/bash
rm -rf shard_test
mkdir shard_test
cd shard_test
head -c 500M </dev/urandom > model.pt
shard init
shard config set chunker_mode rabin
shard add model.pt
shard commit -m "initial checkpoint"
echo "--- DAY 2: Appending 1MB to checkpoint ---"
head -c 1M </dev/urandom >> model.pt
echo "$ time shard add model.pt"
time shard add model.pt
echo "$ time shard commit -m 'epoch 2 checkpoint'"
time shard commit -m "epoch 2 checkpoint"
