using System;
using System.Globalization;
using System.Runtime.InteropServices;
using System.Windows.Forms;

namespace JuleHygge
{
    static class Program
    {
        [DllImport("user32.dll")] static extern bool SetProcessDPIAware();
        [DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr value);
        [DllImport("user32.dll")] static extern IntPtr MonitorFromPoint(System.Drawing.Point pt, uint flags);
        [DllImport("shcore.dll")] static extern int GetDpiForMonitor(IntPtr monitor, int type, out uint dpiX, out uint dpiY);

        // Per-monitor DPI awareness (v2) keeps every coordinate in physical pixels on every screen.
        // With only system awareness, a screen whose DPI differs from the DPI at app start (e.g. external
        // monitors docked after the laptop screen) gets scaled coordinates, while DWM window bounds stay
        // physical, so window snow ended up beside the windows.
        static void SetDpiAwareness()
        {
            try { if (SetProcessDpiAwarenessContext(new IntPtr(-4))) return; } catch { }   // PER_MONITOR_AWARE_V2, Windows 10 1703+
            try { SetProcessDPIAware(); } catch { }
        }

        // The scale factor (DPI / 96) of the monitor that holds the centre of the given screen rectangle.
        public static float DpiScaleAt(System.Drawing.Rectangle r)
        {
            try
            {
                uint x, y;
                var mon = MonitorFromPoint(new System.Drawing.Point(r.X + r.Width / 2, r.Y + r.Height / 2), 2);   // MONITOR_DEFAULTTONEAREST
                if (mon != IntPtr.Zero && GetDpiForMonitor(mon, 0, out x, out y) == 0 && x > 0) return x / 96f;   // MDT_EFFECTIVE_DPI
            }
            catch { }
            using (var g = System.Drawing.Graphics.FromHwnd(IntPtr.Zero)) return g.DpiX / 96f;
        }

        // Test flags run the app directly, without the restart supervisor.
        static readonly string[] TestFlags = { "--kalender", "--julemand", "--sneplov", "--effekt", "--lyd-test" };

        // The exe path at start; captured once because an update renames the running file.
        public static string ExePath;
        public static bool IsChild;

        [DllImport("kernel32.dll", CharSet = CharSet.Unicode)] static extern int GetCurrentPackageFullName(ref int length, System.Text.StringBuilder name);

        // True when the app runs from an MSIX package (the Microsoft Store version, store\make-msix.ps1). The Store
        // then installs updates, the install folder is read-only, and autostart is the package's startup task.
        public static readonly bool IsPackaged = DetectPackage();

        static bool DetectPackage()
        {
            try
            {
                int len = 0;
                return GetCurrentPackageFullName(ref len, null) != 15700;   // APPMODEL_ERROR_NO_PACKAGE
            }
            catch { return false; }   // before Windows 8 there are no packages
        }

        [STAThread]
        static void Main(string[] args)
        {
            ExePath = Application.ExecutablePath;
            // --lyd-test=DIR: write the synthesized sounds as WAV files (to listen to them) and quit.
            foreach (var a in args)
                if (a.StartsWith("--lyd-test=")) { Synth.WriteTestFiles(a.Substring(11)); return; }
            // --sprog-test[=FILE]: write the languages found for the menu, the automatic choice and what each lacks, and quit.
            foreach (var a in args)
                if (a.StartsWith("--sprog-test")) { Lang.Test(a.Length > 13 ? a.Substring(13) : null); return; }
            // --folk-test[=FILE]: write whether this machine's language and region give nisser or elves (and the
            // rule for other locales) and quit. JULE_LOCALE=en_US pretends to be that locale.
            foreach (var a in args)
                if (a.StartsWith("--folk-test")) { Folk.Test(a.Length > 12 ? a.Substring(12) : null); return; }

            IsChild = Environment.GetEnvironmentVariable("JULE_SUPERVISOR") != null;
            bool testRun = false;
            foreach (var a in args) foreach (var f in TestFlags) if (a.StartsWith(f)) testRun = true;
            if (!IsChild && !testRun && Environment.GetEnvironmentVariable("JULE_NO_SUPERVISOR") == null)
            {
                Supervise(args);
                return;
            }
            RunApp(args);
        }

        // The app first starts as a small watchdog that runs the real app as a child process and restarts it if
        // it crashes (after 5 s; it gives up after 3 crashes within a minute of each start). A normal quit
        // ("Afslut") exits with code 0, and then the watchdog quits too. Ported from macos/.../main.swift.
        // Exit code Updater.RestartExitCode means the child installed a new exe: the watchdog then starts the
        // new exe (which becomes the new watchdog) and quits, so both processes run the new version.
        static void Supervise(string[] args)
        {
            var cmd = new System.Text.StringBuilder();
            foreach (var a in args) cmd.Append(cmd.Length > 0 ? " " : "").Append(a.IndexOf(' ') >= 0 ? "\"" + a + "\"" : a);
            if (SuperviseLoop(cmd.ToString()))
            {
                try { System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(ExePath, cmd.ToString()) { UseShellExecute = false }); }
                catch (Exception ex) { ErrorLog.Write(ex); }
            }
        }

        // Returns true when the child asked for a restart after an update (the mutex is released by then).
        static bool SuperviseLoop(string cmd)
        {
            bool createdNew;
            using (var mutex = new System.Threading.Mutex(true, "JuleHygge_Supervisor", out createdNew))
            {
                if (!createdNew) return false;   // already running
                int quickCrashes = 0;
                while (true)
                {
                    var psi = new System.Diagnostics.ProcessStartInfo(ExePath, cmd);
                    psi.UseShellExecute = false;
                    psi.EnvironmentVariables["JULE_SUPERVISOR"] = System.Diagnostics.Process.GetCurrentProcess().Id.ToString(CultureInfo.InvariantCulture);
                    var started = DateTime.Now;
                    int code;
                    try
                    {
                        using (var child = System.Diagnostics.Process.Start(psi))
                        {
                            child.WaitForExit();
                            code = child.ExitCode;
                        }
                    }
                    catch (Exception ex) { ErrorLog.Write(ex); return false; }
                    if (code == 0) return false;   // normal quit
                    if (code == Updater.RestartExitCode) { mutex.ReleaseMutex(); return true; }
                    ErrorLog.Write("JuleHygge stoppede uventet (kode " + code + "); genstarter");
                    quickCrashes = (DateTime.Now - started).TotalSeconds < 60 ? quickCrashes + 1 : 0;
                    if (quickCrashes >= 3) { ErrorLog.Write("supervisor: crasher igen og igen, giver op"); return false; }
                    System.Threading.Thread.Sleep(5000);
                }
            }
        }

        static void RunApp(string[] args)
        {
            bool createdNew;
            using (var mutex = new System.Threading.Mutex(true, "JuleHygge_SingleInstance", out createdNew))
            {
                // right after an update without a supervisor, the old process may still be shutting down
                if (!createdNew && Environment.GetEnvironmentVariable("JULE_AFTER_UPDATE") != null)
                {
                    try { createdNew = mutex.WaitOne(15000); }
                    catch (System.Threading.AbandonedMutexException) { createdNew = true; }
                }
                if (!createdNew) return;
                SetDpiAwareness();
                Application.EnableVisualStyles();
                Application.SetCompatibleTextRenderingDefault(false);
                // A decoration must never pop up crash dialogs: log UI-thread exceptions and keep running.
                Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
                Application.ThreadException += (s, e) => ErrorLog.Write(e.Exception);
                AppDomain.CurrentDomain.UnhandledException += (s, e) => ErrorLog.Write(e.ExceptionObject as Exception);
                var controller = new Controller();
                if (Array.IndexOf(args, "--kalender") >= 0) controller.ShowCalendar();
                // --sneplov sends the snow plow after 3 s; --sneplov=N after N seconds (handy for testing).
                foreach (var a in args)
                {
                    if (a == "--sneplov") controller.SendPlowAfter(3);
                    else if (a.StartsWith("--sneplov="))
                    {
                        float secs;
                        if (float.TryParse(a.Substring(10), NumberStyles.Float, CultureInfo.InvariantCulture, out secs))
                            controller.SendPlowAfter(secs);
                    }
                    // --julemand sends Santa right away, --effekt=N triggers a calendar effect (testing).
                    else if (a == "--julemand") controller.LaunchSanta();
                    else if (a.StartsWith("--effekt="))
                    {
                        int n;
                        if (int.TryParse(a.Substring(9), NumberStyles.Integer, CultureInfo.InvariantCulture, out n)) controller.TriggerEffect(n);
                    }
                }
                Application.Run(controller);
            }
        }
    }

    // Appends errors to %TEMP%\julehygge-error.log, with a cap so a repeating error cannot grow it forever.
    static class ErrorLog
    {
        static int written;

        // With JULE_DEBUG=1, writes a line to %TEMP%\julehygge-debug.log (for testing).
        public static void Debug(string msg)
        {
            if (Environment.GetEnvironmentVariable("JULE_DEBUG") != "1") return;
            try
            {
                System.IO.File.AppendAllText(System.IO.Path.Combine(System.IO.Path.GetTempPath(), "julehygge-debug.log"),
                    DateTime.Now.ToString("HH:mm:ss.fff", CultureInfo.InvariantCulture) + " " + msg + Environment.NewLine);
            }
            catch { }
        }

        public static void Write(string msg) { Write(new Exception(msg)); }

        public static void Write(Exception ex)
        {
            if (ex == null || written >= 200) return;
            written++;
            try
            {
                System.IO.File.AppendAllText(System.IO.Path.Combine(System.IO.Path.GetTempPath(), "julehygge-error.log"),
                    DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss", CultureInfo.InvariantCulture) + " " + ex + Environment.NewLine + Environment.NewLine);
            }
            catch { }
        }
    }

    static class JuleClock
    {
        // Sæt miljøvariablen JULE_DATE=2026-12-10 for at teste julekalender og snemand på en anden dato.
        public static DateTime Today
        {
            get
            {
                string s = Environment.GetEnvironmentVariable("JULE_DATE");
                DateTime d;
                if (!string.IsNullOrEmpty(s) && DateTime.TryParseExact(s, "yyyy-MM-dd", CultureInfo.InvariantCulture, DateTimeStyles.None, out d))
                    return d;
                return DateTime.Today;
            }
        }
    }

    class Flake
    {
        public float X, Y, R, Vy, Phase, Sway, SwaySpd, Rot, RotSpd;
        public int A;
        // Autumn theme: which leaf this is, its shape and colour (see Layer.LeafSprite).
        public int Kind;
        public bool Crystal;
        // About half the flakes can land on a window's top edge (the rest fall past to the ground).
        public bool Catches;
    }

    class Spark
    {
        public float X, Y, VX, VY, Life, Max, Size;
        public bool Gold, Star;
    }

    class Nisse
    {
        public float X, Dir, Speed, WalkPhase, T, StateTimer, Size;
        public int State;          // 0 = går, 1 = står stille, 2 = vinker
        public bool CarriesGift;
        // Walking in from an edge, walking out for good, or walking over to the neighbouring screen.
        public bool Entering, Leaving, Crossing;
        // Meeting another nisse to hand over or swap a gift.
        public float MeetTimer, MeetCooldown;
        public Nisse MeetWith;
        // Magic past the children's snowman: gone for Magic seconds, then back at MagicTo.
        // ZoneBumps counts the turns at the play zone since the last time.
        public float Magic, MagicTo;
        public int ZoneBumps;
        public System.Drawing.Color Tunic, Mitten, Gift;
    }
}
