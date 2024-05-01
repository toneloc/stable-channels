#!/bin/bash

lightning-cli -k plugin subcommand=stop plugin=stablechannels.py

cd /home/clightning

git stash

git pull

chmod +x stablechannels.py
