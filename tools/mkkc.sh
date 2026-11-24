#!/bin/bash
# build the kext collection block (xkc.bin) for the loader.
# extra kcbuild options go in MKKC_OPTS, extra root bundle ids are arguments.
cd ~
python3 ~/xtools/kcbuild.py --kernel ~/anathema-work/x86k/kernel \
  --ext $HOME/kdk3/kdk_files/System/Library/Extensions \
  --ext $HOME/kdk2/kdk_files/System/Library/Extensions \
  --prefer-last com.apple.kec.corecrypto --prefer-last com.apple.kec.Compression \
  --prefer-last com.apple.kec.pthread --prefer-last com.apple.kec.Libm \
  --prefer-last com.apple.kec.AppleEncryptedArchive --prefer-last com.apple.kec.XrtHostedXnu \
  --prefer-last com.apple.security.AppleImage4 \
  --strip com.apple.security.AppleImage4:AppleSecurityExtension \
  --strip com.apple.kext.CoreTrust:AppleSecurityExtension \
  --strip com.apple.driver.AppleMobileFileIntegrity:AppleSecurityExtension \
  --rename com.apple.driver.AppleMobileFileIntegrity=com.apple.kec.AppleMobileFileIntegrity --set-true com.apple.driver.AppleMobileFileIntegrity:AppleKernelExternalComponent \
  --base 0xffffff8002000000 --out $HOME/anathema-work/xkc.bin $MKKC_OPTS \
  com.apple.driver.AppleACPIPlatform com.apple.kec.corecrypto com.apple.kec.AppleEncryptedArchive \
  com.apple.kec.XrtHostedXnu com.apple.kec.Libm com.apple.kec.Compression com.apple.kec.pthread \
  com.apple.security.AppleImage4 com.apple.driver.AppleMobileFileIntegrity com.apple.kext.CoreTrust "$@"
