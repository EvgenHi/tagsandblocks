# tagsandblocks

Statusbar for riverwm

### Building

To build the project, first create build folder with desired name using command[^note]


```sh
meson setup *builddir*
```

and then

```sh
meson compile -C *builddir*
```

---

Ready to use executable will be located in `./*build*/tagsandblocks`

[^note]: Add '--wipe' flag if you want remove existing build directory

## License
This project is licensed under the MIT license
