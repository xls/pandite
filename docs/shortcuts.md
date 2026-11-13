# Keyboard and mouse

The keyboard table below is the shortcut table of the interface
(`SHORTCUTS` in `crates/pandite-ui/src/command.rs`), with the command and key
names that pandite shows. When a command has more than one key, the first
key is the key that tooltips and menus show. Press F1 in pandite to see the
same list.

On macOS, Ctrl is the Command key and Alt is the Option key.

## Keyboard

| Command | Keys |
|---|---|
| Open... | Ctrl+O |
| Save as... | Ctrl+Shift+S, Ctrl+S |
| Copy image | Ctrl+C |
| Copy selected text | Ctrl+C |
| Copy file path | Ctrl+Shift+C |
| Select all text | Ctrl+A |
| Undo | Ctrl+Z |
| Rotate left | Shift+R, L |
| Rotate right | R |
| Flip horizontal | H |
| Flip vertical | V |
| Crop | C |
| Resize... | Ctrl+R |
| Bulk convert and resize... | Ctrl+B |
| Fit to window | F, 0 |
| Actual size (1:1) | 1 |
| Zoom in | +, = |
| Zoom out | - |
| Pan up | Up, Ctrl+Up |
| Pan down | Down, Ctrl+Down |
| Pan left | Ctrl+Left |
| Pan right | Ctrl+Right |
| Previous image | Left, PgUp, Backspace |
| Next image | Right, PgDn, Space |
| First image | Home |
| Last image | End |
| Gallery strip | G |
| Image information | I |
| Full screen | F11, Alt+Enter |
| Context menu | Menu, Shift+F10 |
| Keyboard shortcuts | F1 |
| Close dialog or quit | Esc |
| Quit | Ctrl+Q, Ctrl+W |

### In crop mode

| Command | Keys |
|---|---|
| Apply crop | Enter |
| Select whole image | Ctrl+A |
| Cancel crop | C |
| Move the selection 1 pixel | Left, Right, Up, Down |
| Move the selection 10 pixels | Shift+Left, Shift+Right, Shift+Up, Shift+Down |

### Rules

- The keys +, -, = and the digits ignore Shift, so keyboard layouts that
  need Shift for these keys work.
- When you hold the key, these commands repeat: previous image, next image,
  zoom in, zoom out and the four pan commands. Other commands run one time.
- Ctrl+C copies the selected text when text in the image is selected.
  Otherwise, Ctrl+C copies the image. See [Copy the image](#copy-the-image).
- Ctrl+Shift+C copies the full path of the file. The copy path button on
  the toolbar does the same.
- Ctrl+A and Copy selected text work only on Windows, when pandite has read
  the text of the image. See [text-recognition.md](text-recognition.md).
  On other systems, the shortcut overlay does not show them.
- Esc closes the topmost menu, dialog, question or the shortcut overlay.
  When none of these is open, Esc closes pandite. This is also true in full
  screen, in crop mode, while text is selected, while the image information
  panel shows and while a file loads.
- I shows or hides the image information panel. Shift+I and Ctrl+I do
  nothing. The panel is not a dialog: all other keys work as before. In
  crop mode, the panel hides until the crop ends.
- Esc closes pandite without a question about unsaved edits, the same as
  Ctrl+Q. pandite does not wait for a slow file, a decode, an image copy or
  text recognition.
- A held Esc does not repeat. When it closes a dialog, pandite stays open.
- To leave full screen, press F11 or Alt+Enter. To leave crop mode without
  a crop, press C or click Cancel.
- In a dialog, Full screen and Quit stay available. The dialog receives all
  other keys.
- In crop mode, these commands stay available: fit, 1:1, zoom, pan, full
  screen, gallery strip, image information, context menu, keyboard
  shortcuts, Esc, quit and the crop commands.
- The arrow keys move the selection only when a selection exists.

## Copy the image

Ctrl+C puts the image on the clipboard as pixels. Paste it into an image
editor, a document or a chat.

- The copy has the full resolution of the file, with 8 bits for each color
  channel and alpha.
- The copy includes the rotation, flip and crop that you applied, the same
  pixels that Save as writes. A resize is included too.
- For an animation, the copy is the frame on screen.
- pandite copies in the background. A large image shows "Copying
  image..." first. "Image copied" shows when the image is on the
  clipboard.
- When only a reduced image is on screen because the full resolution is
  still loading, pandite says "Copying when the full image has loaded" and
  copies the full image when it arrives.
- When you press Ctrl+C again before a copy ends, pandite does not start a
  second copy of the same image.
- On Windows, the clipboard gets a 32-bit bitmap. An image with
  transparency up to 16 megapixels also goes on the clipboard as PNG.

## Mouse

| Action | Result |
|---|---|
| Wheel over the image | Zoom at the pointer, 1.2 times for each wheel notch |
| Wheel over the gallery strip | Scroll the strip |
| Drag the image with the left or middle button | Pan |
| Drag from a word with the left button | Select the words from that word to the word nearest the pointer, in reading order |
| Double-click a word | Select the word |
| Triple-click a word | Select the line of the word |
| Click the image away from the text | Clear the text selection |
| Double-click the image away from the text | In fit to window: 1:1 at the pointer. In other modes: fit to window. |
| Right-click | Context menu |
| Back and Forward side buttons | Previous image and next image |
| Click a thumbnail | Show that image |
| Drag the gallery strip | Scroll the strip |

The text actions work only where pandite read text in the image (on
Windows). Over a word, the pointer is a text cursor. The middle button
pans also over text.

### In crop mode

| Action | Result |
|---|---|
| Drag on the image | Draw a new selection |
| Drag inside the selection | Move the selection |
| Drag a handle or an edge | Resize the selection |
| Hold Shift while you drag | Keep a new selection or a corner drag square |
