#!/bin/bash
rm -rf git_test
mkdir git_test
cd git_test
head -c 500M </dev/urandom > model.pt
git init
git lfs install
git lfs track "*.pt"
git add .gitattributes
git add model.pt
git commit -m "initial checkpoint"
echo "--- DAY 2: Appending 1MB to checkpoint ---"
head -c 1M </dev/urandom >> model.pt
echo "$ time git add model.pt"
time git add model.pt
echo "$ time git commit -m 'epoch 2 checkpoint'"
time git commit -m "epoch 2 checkpoint"
