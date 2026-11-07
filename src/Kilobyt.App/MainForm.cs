using System.ComponentModel;
using System.Drawing;
using System.Threading;
using Kilobyt.Core;

namespace Kilobyt.App;

/// <summary>
/// PRD §8 shell. All layout is built in code (no .Designer.cs, no resources) so the
/// project builds in any environment. The form owns no compression logic: it collects
/// settings, calls <see cref="BatchRunner"/>, and renders <see cref="FileProgress"/>.
/// </summary>
public sealed class MainForm : Form
{
    private const int SettingsPanelWidth = 240;
    private const string Dash = "\u2014";

    private readonly BatchRunner _runner;
    private readonly List<Row> _rows = new();

    private Panel _dropPanel = null!;
    private Label _dropTitle = null!;
    private Label _dropSubtitle = null!;
    private Button _addButton = null!;
    private Panel _listHost = null!;
    private ListView _list = null!;
    private Label _emptyLabel = null!;
    private NumericUpDown _targetKb = null!;
    private CheckBox _limitDimensions = null!;
    private NumericUpDown _maxWidth = null!;
    private NumericUpDown _maxHeight = null!;
    private RadioButton _saveSameFolder = null!;
    private RadioButton _saveThisFolder = null!;
    private TextBox _folderBox = null!;
    private Button _browseButton = null!;
    private Button _compressButton = null!;
    private Button _cancelButton = null!;
    private ProgressBar _progressBar = null!;
    private Label _statusLabel = null!;
    private LinkLabel _openFolderLink = null!;
    private ToolTip _toolTip = null!;

    private CancellationTokenSource? _cts;
    private bool _running;
    private int _completed;
    private string? _lastOutputDirectory;

    public MainForm()
    {
        _runner = new BatchRunner(new SystemDrawingJpegCodec());

        AutoScaleDimensions = new SizeF(96F, 96F);
        AutoScaleMode = AutoScaleMode.Dpi;
        Text = "Kilobyt";
        ClientSize = new Size(760, 620);
        MinimumSize = new Size(640, 520);
        StartPosition = FormStartPosition.CenterScreen;
        KeyPreview = true;
        AllowDrop = true;

        BuildUi();
        WireEvents();
        UpdateEmptyState();
        UpdateButtons();
    }

    // ---------------------------------------------------------------- layout

    private void BuildUi()
    {
        var root = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 1,
            RowCount = 3,
            Padding = new Padding(12),
            BackColor = SystemColors.Control,
        };
        root.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100F));
        root.RowStyles.Add(new RowStyle(SizeType.Absolute, 90F));
        root.RowStyles.Add(new RowStyle(SizeType.Percent, 100F));
        root.RowStyles.Add(new RowStyle(SizeType.Absolute, 56F));
        Controls.Add(root);

        root.Controls.Add(BuildDropZone(), 0, 0);
        root.Controls.Add(BuildMiddle(), 0, 1);
        root.Controls.Add(BuildBottomBar(), 0, 2);
    }

    private Control BuildDropZone()
    {
        _dropPanel = new Panel
        {
            Dock = DockStyle.Fill,
            BorderStyle = BorderStyle.FixedSingle,
            BackColor = Color.White,
            Margin = new Padding(0, 0, 0, 8),
            AllowDrop = true,
        };

        var layout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 2,
            RowCount = 2,
            Padding = new Padding(12, 6, 12, 6),
            BackColor = Color.Transparent,
        };
        layout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100F));
        layout.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 140F));
        layout.RowStyles.Add(new RowStyle(SizeType.Percent, 60F));
        layout.RowStyles.Add(new RowStyle(SizeType.Percent, 40F));

        _dropTitle = new Label
        {
            Text = "Drop photos here",
            Font = new Font("Segoe UI", 15.75F, FontStyle.Regular),
            AutoSize = true,
            Anchor = AnchorStyles.Left | AnchorStyles.Bottom,
            Margin = new Padding(0),
        };
        _dropSubtitle = new Label
        {
            Text = "They stay on this PC. Nothing is uploaded.",
            AutoSize = true,
            Anchor = AnchorStyles.Left | AnchorStyles.Top,
            ForeColor = SystemColors.GrayText,
            Margin = new Padding(0),
        };
        _addButton = new Button
        {
            Text = "Add photos\u2026",
            AutoSize = false,
            Size = new Size(132, 32),
            Anchor = AnchorStyles.Left,
            TabIndex = 0,
        };

        layout.Controls.Add(_dropTitle, 0, 0);
        layout.Controls.Add(_dropSubtitle, 0, 1);
        layout.Controls.Add(_addButton, 1, 0);
        layout.SetRowSpan(_addButton, 2);

        _dropPanel.Controls.Add(layout);

        HookDropTarget(_dropPanel);
        HookDropTarget(layout);
        HookDropTarget(_dropTitle);
        HookDropTarget(_dropSubtitle);

        return _dropPanel;
    }

    /// <summary>Makes a control (and the drop zone's children) accept photo drops.</summary>
    private void HookDropTarget(Control control)
    {
        control.AllowDrop = true;
        control.DragEnter += OnDragEnter;
        control.DragOver += OnDragOver;
        control.DragLeave += (_, _) => SetDropHighlight(false);
        control.DragDrop += OnDragDrop;
    }

    private Control BuildMiddle()
    {
        var middle = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 2,
            RowCount = 1,
            Margin = new Padding(0),
        };
        middle.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100F));
        middle.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, SettingsPanelWidth));
        middle.RowStyles.Add(new RowStyle(SizeType.Percent, 100F));

        _listHost = new Panel { Dock = DockStyle.Fill, Margin = new Padding(0, 0, 8, 0) };

        _list = new ListView
        {
            Dock = DockStyle.Fill,
            View = View.Details,
            FullRowSelect = true,
            MultiSelect = true,
            HideSelection = false,
            ShowItemToolTips = true,
            HeaderStyle = ColumnHeaderStyle.Nonclickable,
            BorderStyle = BorderStyle.FixedSingle,
            AllowDrop = true,
            TabIndex = 1,
        };
        _list.Columns.Add("File", 230, HorizontalAlignment.Left);
        _list.Columns.Add("Size", 80, HorizontalAlignment.Right);
        _list.Columns.Add("Result", 150, HorizontalAlignment.Left);
        _list.Columns.Add("Status", 240, HorizontalAlignment.Left);

        _emptyLabel = new Label
        {
            Dock = DockStyle.Fill,
            Text = "No photos yet. Drag some in.",
            TextAlign = ContentAlignment.MiddleCenter,
            ForeColor = SystemColors.GrayText,
            BackColor = SystemColors.Window,
        };

        _listHost.Controls.Add(_list);
        _listHost.Controls.Add(_emptyLabel);
        _emptyLabel.BringToFront();

        middle.Controls.Add(_listHost, 0, 0);
        middle.Controls.Add(BuildSettingsPanel(), 1, 0);
        return middle;
    }

    private Control BuildSettingsPanel()
    {
        var flow = new FlowLayoutPanel
        {
            Dock = DockStyle.Fill,
            FlowDirection = FlowDirection.TopDown,
            WrapContents = false,
            AutoScroll = true,
            Padding = new Padding(4, 4, 4, 4),
            Margin = new Padding(0),
        };

        flow.Controls.Add(Header("Max size"));

        var kbRow = new FlowLayoutPanel { AutoSize = true, WrapContents = false, Margin = new Padding(0, 0, 0, 8) };
        _targetKb = new NumericUpDown
        {
            Minimum = 1,
            Maximum = 99999,
            Value = 100,
            Width = 90,
            TabIndex = 2,
        };
        kbRow.Controls.Add(_targetKb);
        kbRow.Controls.Add(new Label { Text = "KB", AutoSize = true, Margin = new Padding(6, 6, 0, 0) });
        flow.Controls.Add(kbRow);

        _limitDimensions = new CheckBox
        {
            Text = "Limit dimensions",
            AutoSize = true,
            Checked = false,
            Margin = new Padding(0, 0, 0, 6),
            TabIndex = 3,
        };
        flow.Controls.Add(_limitDimensions);

        var dimRow = new FlowLayoutPanel { AutoSize = true, WrapContents = false, Margin = new Padding(0, 0, 0, 12) };
        _maxWidth = new NumericUpDown
        {
            Minimum = 16,
            Maximum = 20000,
            Value = 1600,
            Width = 64,
            Enabled = false,
            TabIndex = 4,
        };
        _maxHeight = new NumericUpDown
        {
            Minimum = 16,
            Maximum = 20000,
            Value = 1600,
            Width = 64,
            Enabled = false,
            TabIndex = 5,
        };
        dimRow.Controls.Add(new Label { Text = "W", AutoSize = true, Margin = new Padding(0, 6, 4, 0) });
        dimRow.Controls.Add(_maxWidth);
        dimRow.Controls.Add(new Label { Text = "H", AutoSize = true, Margin = new Padding(8, 6, 4, 0) });
        dimRow.Controls.Add(_maxHeight);
        flow.Controls.Add(dimRow);

        flow.Controls.Add(Header("Save to"));

        _saveSameFolder = new RadioButton
        {
            Text = "Same folder as original",
            AutoSize = true,
            Checked = true,
            Margin = new Padding(0, 0, 0, 4),
            TabIndex = 6,
        };
        _saveThisFolder = new RadioButton
        {
            Text = "This folder:",
            AutoSize = true,
            Margin = new Padding(0, 0, 0, 4),
            TabIndex = 7,
        };
        flow.Controls.Add(_saveSameFolder);
        flow.Controls.Add(_saveThisFolder);

        var folderRow = new FlowLayoutPanel { AutoSize = true, WrapContents = false, Margin = new Padding(0, 0, 0, 0) };
        _folderBox = new TextBox
        {
            Width = 130,
            ReadOnly = true,
            Enabled = false,
            TabIndex = 8,
        };
        _browseButton = new Button
        {
            Text = "Browse\u2026",
            Width = 82,
            Enabled = false,
            TabIndex = 9,
            Margin = new Padding(4, 0, 0, 0),
        };
        folderRow.Controls.Add(_folderBox);
        folderRow.Controls.Add(_browseButton);
        flow.Controls.Add(folderRow);

        _toolTip = new ToolTip
        {
            AutoPopDelay = 12000,
            InitialDelay = 300,
            ShowAlways = true,
        };
        _toolTip.SetToolTip(_targetKb, "1 KB = 1000 bytes. The app guarantees the file is at or under this.");
        _toolTip.SetToolTip(_limitDimensions, "Never upscale. Output width and height stay within these values.");
        _toolTip.SetToolTip(_saveSameFolder, "Each output is written next to its source photo.");
        _toolTip.SetToolTip(_saveThisFolder, "All outputs go into one folder (flat, not mirrored).");

        return flow;
    }

    private Control BuildBottomBar()
    {
        var bar = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 5,
            RowCount = 1,
            Margin = new Padding(0, 8, 0, 0),
        };
        bar.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 112F));
        bar.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 100F));
        bar.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 200F));
        bar.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100F));
        bar.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 96F));
        bar.RowStyles.Add(new RowStyle(SizeType.Percent, 100F));

        _compressButton = new Button
        {
            Text = "Compress",
            Size = new Size(104, 32),
            Anchor = AnchorStyles.Left,
            TabIndex = 10,
            Enabled = false,
        };
        _cancelButton = new Button
        {
            Text = "Cancel",
            Size = new Size(92, 32),
            Anchor = AnchorStyles.Left,
            TabIndex = 11,
            Visible = false,
        };
        _progressBar = new ProgressBar
        {
            Minimum = 0,
            Maximum = 1,
            Value = 0,
            Height = 18,
            Anchor = AnchorStyles.Left | AnchorStyles.Right,
            Margin = new Padding(0, 0, 12, 0),
        };
        _statusLabel = new Label
        {
            Text = "Ready",
            Dock = DockStyle.Fill,
            TextAlign = ContentAlignment.MiddleLeft,
            AutoEllipsis = true,
            Margin = new Padding(0),
        };
        _openFolderLink = new LinkLabel
        {
            Text = "Open folder",
            AutoSize = true,
            Anchor = AnchorStyles.Right,
            Enabled = false,
            TabIndex = 12,
            Margin = new Padding(8, 0, 0, 0),
        };

        bar.Controls.Add(_compressButton, 0, 0);
        bar.Controls.Add(_cancelButton, 1, 0);
        bar.Controls.Add(_progressBar, 2, 0);
        bar.Controls.Add(_statusLabel, 3, 0);
        bar.Controls.Add(_openFolderLink, 4, 0);

        return bar;
    }

    private static Label Header(string text) => new()
    {
        Text = text,
        AutoSize = true,
        Font = new Font("Segoe UI", 9F, FontStyle.Bold),
        Margin = new Padding(0, 0, 0, 4),
    };

    // ------------------------------------------------------------- behaviour

    private void WireEvents()
    {
        _addButton.Click += (_, _) => AddPhotos();
        _compressButton.Click += (_, _) => StartCompression();
        _cancelButton.Click += (_, _) => CancelRun();
        _openFolderLink.LinkClicked += (_, _) => OpenOutputFolder();
        _browseButton.Click += (_, _) => BrowseForFolder();
        _limitDimensions.CheckedChanged += (_, _) =>
        {
            bool on = _limitDimensions.Checked;
            _maxWidth.Enabled = on && !_running;
            _maxHeight.Enabled = on && !_running;
        };
        _saveThisFolder.CheckedChanged += (_, _) => UpdateButtons();
        _saveSameFolder.CheckedChanged += (_, _) => UpdateButtons();
        _list.Resize += (_, _) => ResizeColumns();
        HookDropTarget(_list);
    }

    private void OnDragEnter(object? sender, DragEventArgs e)
    {
        if (_running || e.Data is null || !e.Data.GetDataPresent(DataFormats.FileDrop)) return;
        e.Effect = DragDropEffects.Copy;
        SetDropHighlight(true);
    }

    private void OnDragOver(object? sender, DragEventArgs e)
    {
        if (_running || e.Data is null || !e.Data.GetDataPresent(DataFormats.FileDrop)) return;
        e.Effect = DragDropEffects.Copy;
        SetDropHighlight(true);
    }

    private void OnDragDrop(object? sender, DragEventArgs e)
    {
        SetDropHighlight(false);
        if (_running || e.Data is null) return;
        if (e.Data.GetData(DataFormats.FileDrop) is string[] files) AddFiles(files);
    }

    private void SetDropHighlight(bool on)
        => _dropPanel.BackColor = on ? Color.FromArgb(226, 240, 253) : Color.White;

    private void AddPhotos()
    {
        if (_running) return;
        using var dialog = new OpenFileDialog
        {
            Title = "Add photos",
            Multiselect = true,
            CheckFileExists = true,
            Filter = "Images (*.jpg;*.jpeg;*.png;*.bmp)|*.jpg;*.jpeg;*.png;*.bmp|All files (*.*)|*.*",
        };
        if (dialog.ShowDialog(this) == DialogResult.OK) AddFiles(dialog.FileNames);
    }

    private void AddFiles(IEnumerable<string> files)
    {
        if (_running) return;

        var known = new HashSet<string>(_rows.Select(r => r.Path), StringComparer.OrdinalIgnoreCase);
        int added = 0;

        foreach (string file in files)
        {
            if (string.IsNullOrWhiteSpace(file)) continue;
            string full;
            try
            {
                full = Path.GetFullPath(file);
            }
            catch (Exception)
            {
                continue;
            }

            if (!known.Add(full)) continue;
            if (Directory.Exists(full)) continue;

            long size = 0;
            try
            {
                var info = new FileInfo(full);
                if (info.Exists) size = info.Length;
            }
            catch (IOException)
            {
            }
            catch (UnauthorizedAccessException)
            {
            }

            var item = new ListViewItem(Path.GetFileName(full))
            {
                ToolTipText = full,
                Tag = full,
            };
            item.SubItems.Add(BatchRunner.FormatKb(size));
            item.SubItems.Add(Dash);
            item.SubItems.Add("Queued");

            _rows.Add(new Row(full, item));
            _list.Items.Add(item);
            added++;
        }

        if (added > 0)
        {
            UpdateEmptyState();
            UpdateButtons();
        }
    }

    private void RemoveSelected()
    {
        if (_running) return;
        if (_list.SelectedItems.Count == 0) return;

        foreach (ListViewItem item in _list.SelectedItems.Cast<ListViewItem>().ToList())
        {
            _rows.RemoveAll(r => ReferenceEquals(r.Item, item));
            _list.Items.Remove(item);
        }

        UpdateEmptyState();
        UpdateButtons();
    }

    private void BrowseForFolder()
    {
        if (_running) return;
        using var dialog = new FolderBrowserDialog
        {
            Description = "Where should the compressed photos go?",
            UseDescriptionForTitle = true,
        };
        if (dialog.ShowDialog(this) == DialogResult.OK)
        {
            _folderBox.Text = dialog.SelectedPath;
            _saveThisFolder.Checked = true;
            UpdateButtons();
        }
    }

    private bool TryReadSettings(out FitSettings settings, out string? outputDirectory)
    {
        settings = new FitSettings(0, null, null);
        outputDirectory = null;

        int target = (int)_targetKb.Value;
        if (target < 1 || target > 99999) return false;

        int? width = null;
        int? height = null;
        if (_limitDimensions.Checked)
        {
            int w = (int)_maxWidth.Value;
            int h = (int)_maxHeight.Value;
            if (w < 16 || w > 20000 || h < 16 || h > 20000) return false;
            width = w;
            height = h;
        }

        if (_saveThisFolder.Checked)
        {
            string folder = _folderBox.Text.Trim();
            if (folder.Length == 0 || !Directory.Exists(folder)) return false;
            outputDirectory = folder;
        }

        settings = new FitSettings(target, width, height);
        return true;
    }

    private async void StartCompression()
    {
        if (_running || _rows.Count == 0) return;

        if (!TryReadSettings(out FitSettings settings, out string? outputDirectory))
        {
            MessageBox.Show(this,
                "Choose an existing output folder (or switch to \u201cSame folder as original\u201d).",
                "Kilobyt",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var paths = _rows.Select(r => r.Path).ToList();
        foreach (Row row in _rows)
        {
            row.Item.SubItems[2].Text = Dash;
            row.Item.SubItems[3].Text = "Queued";
        }

        var cts = new CancellationTokenSource();
        _cts = cts;
        _completed = 0;
        _progressBar.Maximum = Math.Max(1, paths.Count);
        _progressBar.Value = 0;
        _statusLabel.Text = $"Compressing 0 of {paths.Count}\u2026";
        SetRunning(true);

        var progress = new MarshalProgress(this, OnFileProgress);
        CancellationToken token = cts.Token;
        BatchSummary? summary = null;

        try
        {
            summary = await Task.Run(
                () => _runner.RunAsync(paths, settings, outputDirectory, progress, token), token);
        }
        catch (OperationCanceledException)
        {
            _statusLabel.Text = $"Stopped \u2014 {_completed} of {paths.Count} done";
        }
        catch (Exception)
        {
            _statusLabel.Text = "Something went wrong. No photo was left half-written.";
        }
        finally
        {
            SetRunning(false);
            if (ReferenceEquals(_cts, cts)) _cts = null;
            cts.Dispose();
        }

        // The modal belongs after the run is fully closed out.
        if (summary is not null) Finish(summary);
    }

    private void OnFileProgress(FileProgress update)
    {
        if (update.Index < 0 || update.Index >= _rows.Count) return;

        ListViewItem item = _rows[update.Index].Item;
        item.SubItems[2].Text = update.ResultText;
        item.SubItems[3].Text = update.StatusText;

        if (update.State != FileState.Working)
        {
            _completed++;
            if (_progressBar.Value < _progressBar.Maximum) _progressBar.Value++;
            if (update.OutputPath is not null)
                _lastOutputDirectory = Path.GetDirectoryName(update.OutputPath);
            _openFolderLink.Enabled = _lastOutputDirectory is not null;
        }

        if (_running)
            _statusLabel.Text = $"Compressing {_completed} of {_rows.Count}\u2026";
    }

    private void Finish(BatchSummary summary)
    {
        if (summary.Cancelled > 0)
        {
            _statusLabel.Text = $"Stopped \u2014 {summary.Done} of {summary.Total} done";
        }
        else if (summary.Failed > 0)
        {
            _statusLabel.Text = $"{summary.Done} of {summary.Total} done \u00b7 {summary.Failed} failed";
        }
        else
        {
            string kb = BatchRunner.FormatKb((long)_targetKb.Value * 1000L);
            _statusLabel.Text = $"{summary.Done} of {summary.Total} done \u00b7 all at or under {kb} KB";
        }

        if (summary.LastOutputDirectory is not null)
        {
            _lastOutputDirectory = summary.LastOutputDirectory;
            _openFolderLink.Enabled = true;
        }

        if (summary.Failures.Count > 0) ShowFailures(summary.Failures);
    }

    private void ShowFailures(IReadOnlyList<string> failures)
    {
        using var dialog = new Form
        {
            Text = "Some photos could not be compressed",
            ClientSize = new Size(560, 340),
            MinimumSize = new Size(420, 240),
            StartPosition = FormStartPosition.CenterParent,
            MinimizeBox = false,
            ShowIcon = false,
            ShowInTaskbar = false,
            FormBorderStyle = FormBorderStyle.Sizable,
        };

        var layout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 2,
            RowCount = 2,
            Padding = new Padding(12),
        };
        layout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100F));
        layout.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 96F));
        layout.RowStyles.Add(new RowStyle(SizeType.Percent, 100F));
        layout.RowStyles.Add(new RowStyle(SizeType.Absolute, 44F));

        var list = new ListBox
        {
            Dock = DockStyle.Fill,
            IntegralHeight = false,
            HorizontalScrollbar = true,
            SelectionMode = SelectionMode.MultiExtended,
        };
        foreach (string line in failures) list.Items.Add(line);

        string text = string.Join(Environment.NewLine, failures);

        var copyButton = new Button
        {
            Text = "Copy",
            Dock = DockStyle.Top,
            Height = 32,
            Margin = new Padding(8, 0, 0, 0),
        };
        copyButton.Click += (_, _) =>
        {
            try
            {
                Clipboard.SetText(text);
            }
            catch (System.Runtime.InteropServices.ExternalException)
            {
                // Clipboard busy: the list is still selectable by hand.
            }
        };

        var closeButton = new Button
        {
            Text = "Close",
            Dock = DockStyle.Right,
            Width = 96,
            DialogResult = DialogResult.OK,
        };

        layout.Controls.Add(list, 0, 0);
        layout.Controls.Add(copyButton, 1, 0);
        layout.Controls.Add(closeButton, 1, 1);

        dialog.Controls.Add(layout);
        dialog.AcceptButton = closeButton;
        dialog.CancelButton = closeButton;
        dialog.ShowDialog(this);
    }

    private void OpenOutputFolder()
    {
        if (_lastOutputDirectory is null) return;
        try
        {
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo
            {
                FileName = _lastOutputDirectory,
                UseShellExecute = true,
            });
        }
        catch (Win32Exception)
        {
            MessageBox.Show(this, "Could not open the folder.", "Kilobyt", MessageBoxButtons.OK, MessageBoxIcon.Information);
        }
    }

    private void CancelRun()
    {
        if (!_running || _cts is null) return;
        _cancelButton.Enabled = false;
        _statusLabel.Text = "Stopping\u2026";
        try
        {
            _cts.Cancel();
        }
        catch (ObjectDisposedException)
        {
        }
    }

    private void SetRunning(bool running)
    {
        _running = running;

        _addButton.Enabled = !running;
        _targetKb.Enabled = !running;
        _limitDimensions.Enabled = !running;
        _maxWidth.Enabled = !running && _limitDimensions.Checked;
        _maxHeight.Enabled = !running && _limitDimensions.Checked;
        _saveSameFolder.Enabled = !running;
        _saveThisFolder.Enabled = !running;
        _folderBox.Enabled = !running && _saveThisFolder.Checked;
        _browseButton.Enabled = !running && _saveThisFolder.Checked;

        _cancelButton.Visible = running;
        _cancelButton.Enabled = running;
        _compressButton.Enabled = !running && _rows.Count > 0;
        _openFolderLink.Enabled = !running && _lastOutputDirectory is not null;
    }

    private void UpdateButtons()
    {
        bool busy = _running;
        _compressButton.Enabled = !busy && _rows.Count > 0;
        _addButton.Enabled = !busy;
        bool manual = _saveThisFolder.Checked && !busy;
        _folderBox.Enabled = manual;
        _browseButton.Enabled = manual;
        _maxWidth.Enabled = !busy && _limitDimensions.Checked;
        _maxHeight.Enabled = !busy && _limitDimensions.Checked;
        _openFolderLink.Enabled = !busy && _lastOutputDirectory is not null;
    }

    private void UpdateEmptyState()
    {
        bool empty = _rows.Count == 0;
        _emptyLabel.Visible = empty;
        _list.Visible = true;
        if (empty) _emptyLabel.BringToFront();
    }

    private void ResizeColumns()
    {
        if (_list.Columns.Count != 4) return;
        int total = _list.ClientSize.Width;
        int file = Math.Max(160, (int)(total * 0.36));
        int size = 80;
        int result = Math.Max(130, (int)(total * 0.24));
        int status = Math.Max(140, total - file - size - result - 4);
        _list.Columns[0].Width = file;
        _list.Columns[1].Width = size;
        _list.Columns[2].Width = result;
        _list.Columns[3].Width = status;
    }

    // --------------------------------------------------------------- keyboard

    protected override bool ProcessCmdKey(ref Message msg, Keys keyData)
    {
        if (keyData == (Keys.Control | Keys.O))
        {
            AddPhotos();
            return true;
        }

        if (keyData == Keys.Escape)
        {
            CancelRun();
            return true;
        }

        bool editing = ActiveControl is TextBoxBase || ActiveControl is NumericUpDown || ActiveControl is UpDownBase;

        if (keyData == Keys.Delete && !editing)
        {
            RemoveSelected();
            return true;
        }

        if (keyData == Keys.Enter && !editing)
        {
            StartCompression();
            return true;
        }

        return base.ProcessCmdKey(ref msg, keyData);
    }

    protected override void OnShown(EventArgs e)
    {
        base.OnShown(e);
        ResizeColumns();
    }

    private sealed class Row
    {
        public Row(string path, ListViewItem item)
        {
            Path = path;
            Item = item;
        }

        public string Path { get; }
        public ListViewItem Item { get; }
    }
}

/// <summary>
/// Marshals <see cref="FileProgress"/> onto the UI thread: posts through the captured
/// <see cref="SynchronizationContext"/>, falling back to <see cref="Control.BeginInvoke"/>.
/// </summary>
internal sealed class MarshalProgress : IProgress<FileProgress>
{
    private readonly Control _form;
    private readonly SynchronizationContext? _context;
    private readonly Action<FileProgress> _handler;

    public MarshalProgress(Control form, Action<FileProgress> handler)
    {
        _form = form;
        _context = SynchronizationContext.Current;
        _handler = handler;
    }

    public void Report(FileProgress value)
    {
        if (_context is not null)
        {
            _context.Post(_ => _handler(value), null);
            return;
        }

        if (_form.IsHandleCreated && _form.InvokeRequired)
        {
            _form.BeginInvoke(new Action(() => _handler(value)));
            return;
        }

        _handler(value);
    }
}
