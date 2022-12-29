#!/bin/bash
set -exuo pipefail

mkdir -p model_data


MODELS="sentence-transformers/msmarco-distilbert-base-tas-b sentence-transformers/msmarco-distilbert-dot-v5 sentence-transformers/msmarco-bert-base-dot-v5"

git lfs install --skip-repo

cd model_data
BASE_DIR=`pwd`

if [ ! -d "rust-bert" ]; then
  git clone https://github.com/guillaume-be/rust-bert.git
fi

for path in $MODELS; do
  IFS=/ read -a array <<< "$path"
  REPO_ORG=${array[0]}
  MODEL=${array[1]}

  cd $BASE_DIR
  if [ -d "$MODEL" ]; then
    continue;
  fi

  git clone https://huggingface.co/$REPO_ORG/$MODEL
  cd $MODEL

  CONVERT_ARGS=
  if [[ $MODEL == *"distilbert"* ]]; then
    CONVERT_ARGS="--prefix distilbert."
  fi

  python3 $BASE_DIR/rust-bert/utils/convert_model.py pytorch_model.bin $CONVERT_ARGS
done

