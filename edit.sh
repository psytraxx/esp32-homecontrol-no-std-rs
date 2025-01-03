#!/bin/bash

. ~/export-esp.sh

# Run cargo in release mode
export $(cat .env | xargs) && code .