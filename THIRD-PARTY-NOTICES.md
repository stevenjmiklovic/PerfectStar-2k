# Third-party notices

PerfectStar 2k bundles the `en_US` Hunspell spellchecking dictionary
(`assets/en_US.aff`, `assets/en_US.dic`), sourced from the
[LibreOffice dictionaries](https://github.com/LibreOffice/dictionaries)
project (`en/en_US.aff`, `en/en_US.dic`, SCOWL size 60, built 2020-12-07).
The full upstream README is kept alongside it at
`assets/README_en_US.txt`.

## Word list — SCOWL / Kevin Atkinson

The word list is derived from [SCOWL](http://wordlist.sourceforge.net/),
whose collective work is:

> Copyright 2000-2018 by Kevin Atkinson
>
> Permission to use, copy, modify, distribute and sell these word
> lists, the associated scripts, the output created from the scripts,
> and its documentation for any purpose is hereby granted without fee,
> provided that the above copyright notice appears in all copies and
> that both that copyright notice and this permission notice appear in
> supporting documentation. Kevin Atkinson makes no representations
> about the suitability of this array for any purpose. It is provided
> "as is" without express or implied warranty.

SCOWL incorporates several public-domain sources (the Moby lexicon
project, Brian Kelk's UK English Wordlist, the 12Dicts package) and the
WordNet inflection database (Copyright 1997 Princeton University,
provided under a permissive license — see `assets/README_en_US.txt` for
its full text).

## Affix rules — Geoff Kuenning / Ispell

The affix file is a modified version of the `english.aff` file from
Geoff Kuenning's Ispell, under a modified BSD license:

> Copyright 1993, Geoff Kuenning, Granada Hills, CA
> All rights reserved.
>
> Redistribution and use in source and binary forms, with or without
> modification, are permitted provided that the following conditions
> are met:
>
> 1. Redistributions of source code must retain the above copyright
>    notice, this list of conditions and the following disclaimer.
> 2. Redistributions in binary form must reproduce the above copyright
>    notice, this list of conditions and the following disclaimer in the
>    documentation and/or other materials provided with the distribution.
> 3. All modifications to the source code must be clearly marked as
>    such. Binary redistributions based on modified source code
>    must be clearly marked as modified versions in the documentation
>    and/or other materials provided with the distribution.
> (clause 4 removed with permission from Geoff Kuenning)
> 5. The name of Geoff Kuenning may not be used to endorse or promote
>    products derived from this software without specific prior
>    written permission.
>
> THIS SOFTWARE IS PROVIDED BY GEOFF KUENNING AND CONTRIBUTORS "AS IS" AND
> ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
> IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
> ARE DISCLAIMED. IN NO EVENT SHALL GEOFF KUENNING OR CONTRIBUTORS BE LIABLE
> FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
> DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS
> OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION)
> HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT
> LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY
> OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF
> SUCH DAMAGE.

## Spellbook

Spellchecking itself is performed by the [`spellbook`](https://crates.io/crates/spellbook)
crate, used under its own license terms as declared on crates.io.

## Similar

Revision diffing (the snapshot comparison view) uses the
[`similar`](https://crates.io/crates/similar) crate, dual-licensed
Apache-2.0 OR MIT, used under its license terms as declared on crates.io.
See `docs/adr/ADR-014-similar-crate-for-revision-diff.md` for why this
dependency was adopted.
