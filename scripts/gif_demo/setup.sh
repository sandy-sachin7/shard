#!/bin/bash
echo "Setting up real test environment..."

# Clean up previous runs
rm -rf git_test shard_test

# Setup Git directory
mkdir git_test
cd git_test
head -c 500M </dev/urandom > model.pt
cd ..

# Setup Shard directory
mkdir shard_test
cd shard_test
head -c 500M </dev/urandom > model.pt
cd ..

echo "Test environments 'git_test' and 'shard_test' created with 500MB model files."
echo "You can now run 'vhs < demo.tape' to record the actual real-time execution!"
