# My Sushi patches

Patches I made with Sushi. Every file is a `sushi-patch` JSON document with a
`format`, a `name`, a `version` and a `params` object that holds the parameter
values by their stable IDs.

The format is forward and backward compatible: a parameter that a file does not
contain keeps the factory default, and a parameter that the file contains but
the plugin does not know is ignored. That is why these patches keep loading even
though they were written by different builds.

## Patches

| File | Patch |
| --- | --- |
| `bad.json` | bad |
| `basic noise.json` | basic noise |
| `clean key.json` | clean key |
| `cool noise.json` | cool noise |
| `dance.json` | dance |
| `Flow1.json` | Flow1 |
| `flow2.json` | flow2 |
| `JUNO BASS arp.json` | JUNO BASS arp |
| `warm key.json` | warm key |

## Use this folder in the plugin

The plugin stores its patch folder in `$XDG_CONFIG_HOME/sushi/config.json`
(`~/.config/sushi/config.json` by default):

```json
{
  "patch_dir": "/home/shienze/Projects/sushi/patches"
}
```

The same folder can be set from the patch browser in the editor. On another
machine, use the path of that machine's checkout.

## Save a patch to this repository

1. Save the patch from the patch browser (it is written into this folder).
2. Run `../scripts/sync-patches.sh` to commit and push it:

```bash
scripts/sync-patches.sh
```

The script only stages `patches/`, so unrelated work in the checkout is never
committed with it.
