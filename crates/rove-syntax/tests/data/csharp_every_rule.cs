// 这份 C# 只用于 #117 的「遍历结果必须等于高亮查询结果」比对，不参与构建。
// 目标是把 queries/csharp/highlights.scm 里的每一条规则都走到一次。
using System;
using System.Collections.Generic;
using Alias = System.Text.StringBuilder;
using static System.Math;

namespace Fixture.Every.Rule
{
    public delegate int Transform(int value);

    public record Point(int X, int Y);

    public enum Mode
    {
        Idle,
        Running = 2,
    }

    file class HiddenHelper
    {
        /* 块注释 */
        internal const string Tag = @"C:\temp\verbatim";
    }

    public interface IRunnable
    {
        void Run();
    }

    public abstract class Base : IRunnable, IDisposable
    {
        public abstract void Run();

        public void Dispose() { }

        public virtual void Reset() { }
    }

    public unsafe struct Pixels
    {
        public fixed char Data[8];
    }

    [Serializable]
    [System.Obsolete("过时", false)]
    public sealed partial class Sample<T, TOther> : Base, IComparable<Sample<T, TOther>>
        where T : class
    {
        // 独占一行的注释，前导空白要和注释同色。
        private readonly List<int> values = new List<int>();
        private readonly List<Point> points = new List<Point>();
        private volatile int counter;
        protected internal static readonly string Name = "sample";
        public const int Limit = 10;
        private const double Ratio = 1.5;
        private const int Mask = 0xFF;
        private const int Flags = 0b1010;
        private const char Marker = '\n';
        private const string Escaped = "line\tbreak\n";
        private const string Raw = """raw text""";

        public event EventHandler Changed
        {
            add { Changed += value; }
            remove { Changed -= value; }
        }

        public bool IsActive { get; private set; }

        public required string Label { get; init; }

        public int Count => values.Count;

        public Sample(int start)
        {
            counter = start;
            IsActive = true;
        }

        public override void Run()
        {
            Reset();
        }

        public new void Reset()
        {
            counter = 0;
        }

        public static extern void Native();

        public static Sample<T, TOther> operator +(Sample<T, TOther> left, Sample<T, TOther> right)
        {
            return left;
        }

        public async System.Threading.Tasks.Task<int> Compute(int seed, object boxed)
        {
            await System.Threading.Tasks.Task.Yield();
            var local = seed;
            int doubled = local * 2;
            long widened = doubled;
            short narrow = 1;
            ushort unsignedNarrow = 1;
            uint unsignedWord = 1;
            ulong unsignedWide = 1;
            byte small = 1;
            sbyte signedSmall = 1;
            float ratio = 0.5f;
            double precise = Ratio;
            decimal money = 1.25m;
            nint native = 0;
            nuint nativeUnsigned = 0;
            bool flag = local > 0 && local < Limit || !IsActive;
            string text = boxed as string;
            Transform transform = value => value + 1;
            object fallback = default(object);
            void Helper(int inner)
            {
                counter += inner;
            }

            Helper(local);
            lock (values)
            {
                values.Add(local);
            }

            using (var buffer = new System.IO.MemoryStream())
            {
                buffer.Flush();
            }

            foreach (var item in values)
            {
                if (item == doubled)
                {
                    continue;
                }
                else if (item != widened)
                {
                    break;
                }
            }

            for (int index = 0; index < values.Count; index++)
            {
                counter -= index;
            }

            do
            {
                counter++;
            }
            while (counter <= 0);

            while (counter >= 100)
            {
                counter /= 2;
            }

            switch (local)
            {
                case 1:
                    goto case 2;
                case 2 when flag:
                    counter %= 3;
                    break;
                default:
                    break;
            }

            try
            {
                System.Console.WriteLine($"counter {counter:F2} for {text}");
            }
            catch (InvalidOperationException error) when (error != null)
            {
                throw new InvalidOperationException(Name, error);
            }
            finally
            {
                IsActive = false;
            }

            var first = values[0];
            var picked = flag ? first : doubled;
            var created = Create<int>(picked);
            var length = (local) + this.counter;
            return created + length + (int)Math.Abs(money switch { _ => 0 });
        }

        public static TValue Create<TValue>(TValue seed)
        {
            return seed;
        }

        public System.Collections.Generic.IEnumerable<int> Walk()
        {
            foreach (var value in values)
            {
                yield return value;
            }
        }

        public int CompareTo(Sample<T, TOther> other)
        {
            return 0;
        }
    }
}
