Requirements:
- The executor should not do any heap memory allocations.
- The executor should be single threaded.
- If possible, the executor should be independent of the future backends. For example, it
should be easy to swap out the future backend from a IO Uring based to one that is based off of
SPDK.

An architecture that I saw in the LeanStore author's paper was that they were using green
threads implemented in C++ using the Boost library. I like that idea and I think the same thing
can be done in Rust as well but I do not want green threads because I do not see the benefit of
having them (green threads usually have their own local stack so things like recursion are easier).

Having that said, I still liked the basic idea that they had and it was to have a construct
called "scheduler" which does the following:
1. Process the tasks given to it by running them in the green threads. The tasks can issue IO
   and then yield which will drive the scheduler to run other tasks.
2. Once the tasks are processed once the scheduler would move on to submit the IO requests to
   the backend.
3. As part of the scheduler there are some "system" tasks like cache eviction, etc. which are
   then run in the green threads.
4. Then the scheduler would poll for ready IO and will wake the tasks in step 1 once it loops
   back to them.

Now the idea is to model the same thing in Rust but without the green threads. I think that
will require 2 things:
1. Creating a scheduler which is an future executor.
2. Create multiple futures which are the tasks that the scheduler will run.