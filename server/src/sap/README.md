# SAP signing

Apple rejects unsigned App Store sign-in requests (HTTP 403 / 204 / 301 without
`Location`) since August 2026. Every `authenticate` request must carry an
`X-Apple-ActionSignature` header computed over its exact body.

This module is a Rust port of the browser SAP implementation in
[AssppWeb](https://github.com/Lakr233/AssppWeb) (`frontend/src/apple/sap/`,
revision `485be0b`), which itself ports the direct implementation in
[ApplePackage](https://github.com/Lakr233/ApplePackage) (revision `773583a`).
It contains no Apple executable or private framework.

`tables.json` holds the fixed lookup tables and permutations copied from that
revision; `testdata/sap-handshake.json` holds the synthetic test vectors (no
Apple certificate, real account or captured production session).

When Apple changes the protocol again, check AssppWeb / ApplePackage first and
port their fix.

## License

The ported code and data are used under the MIT license:

```
MIT License

Copyright (c) 2025 Lakr233 with his friends

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
