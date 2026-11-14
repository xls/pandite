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
| Select image | Space |
| Clear selection | Backspace |
| Select and previous image | Shift+Left |
| Select and next image | Shift+Right |
| Move to... | M |
| Move to Recycle Bin (Windows), Move to Trash (macOS, Linux) | Del |
| Delete permanently... | Shift+Del |
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
| Previous image | Left |
| Next image | Right |
| Back one gallery page | PgUp |
| Forward one gallery page | PgDn |
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
  select and previous image, select and next image, back and forward one
  gallery page, zoom in, zoom out and the four pan commands. Other commands
  run one time.
- PgDn and PgUp move by one page of the gallery strip: the number of
  thumbnails that fit whole in the strip at the current window width. When
  the strip is hidden, the page is the number that the strip would show.
  At the first and last image the move stops. When you press the key again
  there, the move wraps to the other end only if single steps wrap.
- Del moves the file of the current image to the Recycle Bin (Windows) or
  the Trash (macOS, Linux) without a question, because you can restore it.
  Then the next image shows, or the previous image if the file was the
  last. See [Delete a file](#delete-a-file).
- When images are selected, Del and Shift+Del act on the selected images
  and ask first. See [Select images](#select-images).
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

## Delete a file

When no image is selected, Del and Shift+Del act on the file of the
current image. When images are selected, they act on the selected images.
See [Delete the selected images](#delete-the-selected-images).

- Del moves the file to the Recycle Bin (Windows) or the Trash (macOS,
  Linux). pandite does not ask first. To get the file back, restore it from
  the Recycle Bin or the Trash.
- Shift+Del deletes the file permanently. You cannot undo this, so pandite
  asks first and names the file. Keep is the default button: Enter or Esc
  keeps the file. To delete it, press Right and then Enter, or click
  Delete.
- After a delete, the next image shows, or the previous image if the file
  was the last. The folder list, the position counter and the gallery strip
  drop the file at once. When the folder has no image left, the window is
  empty.
- Unsaved edits of the image (rotate, flip, crop, resize) go with the file.
- When the delete fails, the image stays and a message at the bottom gives
  the reason. When the file is already gone, pandite drops it from the list
  and shows the next image.
- One key press deletes one file. A held key does not delete more files.
- The keys do nothing while a dialog is open, in crop mode, or while a text
  field has the focus. In a text field, Del deletes text.
- When the drive of the file has no Recycle Bin or Trash, Del deletes the
  file permanently, without a question and without a message.
- On Windows, only fixed drives have a Recycle Bin. A network drive or a
  removable drive has no Recycle Bin. When Windows cannot put a file in the
  Recycle Bin of a fixed drive (the file is too large for it, or it is
  turned off), Windows asks before it deletes the file permanently.
- On Linux, the file goes to the trash in your home folder
  (`$XDG_DATA_HOME/Trash`), or to the trash at the top of its drive when it
  is on another drive. This follows the FreeDesktop Trash specification.

## Select images

Select images to delete, move or convert them together. The selection
changes with the keyboard and the context menu only. The mouse does not
change it.

- Space selects the current image. When the current image is selected,
  Space clears its selection.
- Shift+Right selects the current image and then shows the next image.
  Shift+Left selects the current image and then shows the previous image.
  An image that is selected stays selected. The image that shows next is
  not selected by that key.
- When you press Shift on a selected image, Shift+Right and Shift+Left
  clear the selection of each image they leave. This continues until you
  release Shift.
- Space, Backspace, Shift+Right and Shift+Left show the toolbar. The
  toolbar hides again after the usual idle time.
- Hold Shift+Right or Shift+Left to select a run of images. At the first
  or the last image, the key selects the image and the view stays.
- Backspace clears the selection of all images. When no image is selected,
  Backspace does nothing.
- In the gallery strip, a selected image shows a small round badge with a
  check mark in the top left corner of its thumbnail. Other thumbnails
  show no badge.
- A click on a thumbnail shows that image, also a click on its badge.
- The label "Marked" in the bottom left corner of the image shows that the
  current image is selected.
- The toolbar shows the number of selected images, for example
  "3 selected", next to the position.
- The context menu has Select image and Clear selection.
- The selection holds files of the current folder. It stays when you move
  to other images. A selected file that leaves the folder also leaves the
  selection. When you open a file of another folder, no image is selected.
- Space, Backspace, Shift+Left and Shift+Right do not change the selection
  in crop mode, while a dialog is open and in the keyboard shortcut
  overlay. In a text field, they edit the text. In crop mode, Shift and an
  arrow key move the crop selection 10 pixels. Esc does not clear the
  selection.

### Delete the selected images

- Del asks first and gives the number of images, for example "Move 3
  images to the Recycle Bin?". Move is the default button. To keep the
  images, press Esc or click Cancel.
- When the drive of the images has no Recycle Bin or Trash, Del asks
  "Delete 3 images permanently?". Keep is the default button.
- When only some of the images are on a drive without a Recycle Bin or
  Trash, the question gives their number. These images are deleted
  permanently.
- Shift+Del asks "Delete 3 images permanently?". Keep is the default
  button. To delete the images, press Right and then Enter, or click
  Delete.
- Only the selected images go. The current image goes only when it is
  selected.
- After the delete, the current image stays when it was not selected.
  When it was selected, the next remaining image shows, or the previous
  remaining image when it was the last.
- When a file cannot go, a message at the bottom gives the reason. That
  file stays in the folder and in the selection. The other files go.

### Convert the selected images

- When images are selected, the bulk convert dialog (Ctrl+B) opens with
  the selected images as the source. The job converts only these images.
- To convert other images, click Add files, Add folder or Current folder.
  To use the selected images again, click Selected images.
- When no image is selected, the dialog does not show the Selected images
  button.

## Move images

M moves the selected images to a folder that you choose. When no image is
selected, M moves the current image.

- M opens the folder picker. The picker starts in the folder of the last
  move, or in the current folder. To move nothing, cancel the picker.
- There is no question. To undo the move, move the images back.
- A file in the chosen folder is never replaced. When the chosen folder
  has a file with the same name, that image does not move and stays
  selected.
- When you choose the folder that has the images, nothing moves and a
  message says so.
- On another drive, each file is copied first. The original goes only
  after the copy is complete. When a copy fails, the original stays and
  the incomplete copy is removed.
- After the move, the current image stays when it did not move.
  Otherwise the next remaining image shows, or the previous remaining
  image when it was the last.
- A message gives the number of moved images. When an image does not
  move, the message gives the reason and the number of other images that
  did not move.
- M does nothing in crop mode, while a dialog is open and in the keyboard
  shortcut overlay. In a text field, it types the letter. Without an
  image, Move to is not available.

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
| Double-click a word, keep the button down and drag | Select whole words from that word to the word nearest the pointer, in reading order |
| Triple-click a word, keep the button down and drag | Select whole lines from the line of that word to the line nearest the pointer |
| Click the image away from the text | Clear the text selection |
| Double-click the image away from the text | In fit to window: 1:1 at the pointer. In other modes: fit to window. |
| Right-click | Context menu |
| Back and Forward side buttons | Previous image and next image |
| Click a thumbnail | Show that image (the selection does not change) |
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
